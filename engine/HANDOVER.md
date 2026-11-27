# Handover across OpenVM versions (design, 26.11.2026)

User decision (26.11.2026): build the handover now, so an OpenVM upgrade can never strand the proved chain.

## The problem

The engine's chain is a recursion: block k's aggregate verifies block k-1's proof in-guest (OpenVM deferral,
`env::verify`). That check is made by OpenVM's verify-stark deferral circuit against the aggregation verifying key of
the SAME VM. A minor OpenVM version changes the recursion circuits and therefore that key (VERSIONING.md: only patch
versions keep `MultiStarkVerifyingKey` compatibility). After an upgrade, the new image's deferral circuit cannot check
the old chain's last proof, and the chain would have to restart.

## The design: the previous version's verifier, as software, inside the new guest

OpenVM's host verifier (`openvm-verify-stark-host::verify_vm_stark_proof`) is plain Rust with no OS needs once
`abi_stable` is out (engine/wasm-verify: it builds for wasm32 and accepts real engine proofs). Compiled for the guest
(riscv32, no_std + alloc), it runs inside a zkVM like any other code. So:

1. **Pin.** Each released engine image records its predecessor: the previous version's aggregation verifying key and
   baseline (the `VmStarkVerifyingKey` the wasm verifier takes), its image id (SHA-256(exe || vm)), and the handover
   height H. As RISC Zero's handover_pin.rs does, the pin is part of the new image's source, so it is in its image id.
2. **Handover proof.** At H, the new image's aggregate for block H+1 takes the old chain's proof of block H as plain
   input bytes (not a deferral), runs the PREVIOUS version's verifier on it in software, and checks what the verified
   proof commits: its image id against the pin, its public values against SHA-256(old journal), the old journal
   decoding as ChainState at H. Then it continues the chain from that state, exactly as a recursion step would.
   The check runs ONCE per upgrade, so its cost (a software STARK verification) does not matter for throughput.
3. **Lineage.** The new ChainState carries the lineage (the list of image ids since genesis), as the RISC Zero chain's
   `lineage` field does, so a verifier knows which images it rests on and their order.
4. **Verifiers.** A verifier of the new chain checks the new image's proof only: the handover is inside it. To check
   the old chain itself, the old release's verifier is still the tool.

What this needs from OpenVM, none of it a fork: the verifier crate building for the guest target (the wasm patch's
`abi_stable` removal, upstream or carried), and its cost inside the zkVM, to be measured.

## Alternatives, and why not

- **SNARK wrap (OpenVM's root prover to Halo2/KZG):** cheaper to verify in-guest, but a trusted setup and a second
  proof system on the soundness path; kept as a fallback if the software verifier turns out too costly.
- **Keep the old version's prover forever:** no handover, but every upgrade then forks the chain.

## Plan

1. **DONE (26.11.2026, results/handover_step1):** OpenVM v2.1.0's verifier compiled into an engine guest verifies a
   real engine proof (verified 1, image id and public values as expected) and refuses it with one byte flipped
   (verified 0). Cost 15.6B per check, paid once per upgrade. Needs two small upstreamable patches
   (openvm-verify-nozstd, stark-sdk-nologging) besides the WASM one.
2. The pin and the handover mode in the guest (aggregate input carries the old proof; the checks above), behind a
   build-time pin so an image without a predecessor cannot take that path.

   **Design (26.11.2026 evening), from the product guest's own handover (prover/methods/guest: handover_pin.rs,
   lineage/, the `is_base == 2` step in main.rs).** It already has the slot this needs: a `Handover` entry is
   `{image, first_height, verifier}`, and `verifier` is the zkVM version the predecessor's proofs were made under
   (SAME_ZKVM = zero: `env::verify`; non-zero: an older RISC Zero, checked by its vendored recursion circuit,
   `xver::verify`). The lineage digest already commits `verifier` (`lineage_step`: 101-byte preimage when non-zero), so
   a chain's lineage says which verifier every handover used. For OpenVM:
   - `verifier` = SHA-256 of the previous OpenVM version's verifying key (the bitcode `VmStarkVerifyingKey`: agg vk +
     the predecessor image's baseline, what verify-core takes). The key's bytes are embedded in the image
     (include_bytes, a file per pinned predecessor under the lineage dir) and hashed at run time against the pin, so the
     image id commits to exactly that key;
     ⛔ the pin is REQUIRED, not a convenience (engine/VERIFIER.md, review 26.11 evening): the image id does not bind
     the aggregation key `mvk`, so a key taken from input would let a forged `mvk` pass. Step 1's probe guest reads
     the key from input and commits only image_id_of(key): fine for measuring the cost, NOT a check to build on; the predecessor's image id (`image`) must equal image_id_of(that key)
     (checked at build time and in the guest).
   - The engine's `xver::verify` is the software OpenVM verifier of step 1 (verify_vm_stark_proof_decoded, the
     nozstd + nologging patches), compiled ONLY into an image whose lineage has such an entry (a cargo feature, as
     the RISC Zero `xver-*` features are), so every other image's code and id are unchanged. It is given the old proof
     as plain input words and must establish exactly what `env::verify(p.image, &prev_journal)` would: the proof
     verifies under the pinned key, and its public values are SHA-256(prev_journal).
   - Everything after it is today's handover code, unchanged: `prev.self_id == p.image`, the profile, the pinned
     height, the predecessor's own lineage. A same-OpenVM handover (a rule change on the same VM) needs no new code:
     SAME_ZKVM, `env::verify`, which the engine maps to OpenVM's deferral (tested: zAPPs verify core proofs that way).
   - Host: `engine-export` for the handover block writes the old proof's bytes into the aggregate stream where
     `xver` reads them (`n`, then the words), from the old chain's tip proof (proof.json).
   Cost: 15.6B once per upgrade (step 1), inside the handover block's aggregate.

   **Built (27.11.2026):**
   - Guest: `engine/guest/xver_ovm.rs` behind the engine feature `ovm-xver` (edits.py edit 1 switches the module; the
     switch is on ONE line so the default image stays byte-identical: rebuilt at HEAD, exe b7d7c148 with and without
     it). It carries each predecessor key as an embedded file with its pin (assemble.sh `HZ_OVM_XVER_KEYS`, generated
     src/xver_keys.rs; main.rs's compile-time check `carried(verifier)` passes only for a carried pin) and checks the
     proof with verify-core's `verify_full_bytes` (pin, image id from the key, STARK, public values = SHA-256(journal)).
     With `ovm-xver` the whole guest takes OpenVM from ONE verified patched tree (openvm_src.sh: nozstd, nologging).
   - Test images: `HZ_TEST_LINEAGE` replaces lineage/test.rs in the assembled copy only (the test variant, never
     published); a host of the same variant is built from a copy of the checkout with that file.
   - Host: `HAZYNC_HANDOVER_PROOF=<P>` in libzync-prove-range on the engine: the predecessor's state from P.journal
     (checked against the pin's image and height), P.bin (plain encoding) as words after the tail in the collect and
     prove passes, no child; the coin log / trie next to P (handover_dir()).
   - **Two OpenVM versions in one guest are still ahead (step 3):** across a real upgrade the new guest must compile
     the OLD version's verifier next to its own runtime. Step 2 is rehearsed with a predecessor of the SAME OpenVM
     (keys-ch) forced through the software path.
3. **Sized 27.11.2026 (night):** OpenVM's tags after v2.1.0: only `v2.x.0-preview.1/.2`, which are NOT a target
   (dated 04.09.2026, Cargo version still 2.1.0, stark-backend on `branch = "main"`, unpinned). So the rehearsal runs
   from the previous release, **v2.0.2** (14.08.2026; stark-backend tag v2.0.1; has the deferral extension and
   verify-stark), as the predecessor, to v2.1.0. v2.0.2 -> v2.1.0: extensions/deferral and sdk-config identical;
   guest-libs/verify-stark 18 lines, toolchain/openvm 11, sdk 284, continuations 202, vm 318 lines differ; the
   abi-stable and nozstd patches apply to v2.0.2 as they are. Work: (a) the engine (host, platform, guest build) on
   v2.0.2: retag + the few API changes; keys on v2.0.2; a short chain proved there; (b) the v2.1.0 image's xver built
   against v2.0.2's verifier crate (a second OpenVM in the guest's graph: openvm-verify-stark-host v2.0.2 renamed,
   with its own stark-backend v2.0.1), since an upgrade's xver must run the PREDECESSOR's verifier; (c) the handover
   at the chain's next height with the negative controls.
   **cargo check of hzengine retagged to v2.0.2 / stark-backend v2.0.1 (scratch copy):** 6 errors, all from ONE layout
   change: v2.0.2's VerificationBaseline holds a single `app_exe_commit` FIELD (v2.1.0: program_commit, initial_state,
   initial_pc, and app_exe_commit() computed), and v2.0.2's proof decodes through the codec `Decode` trait (no vk).
   segpar.rs builds the baseline from its parts; engine.rs calls app_exe_commit() and decode_using_vk. So: a small host
   port; and the predecessor's key file is NOT the same struct across versions, so xver must use the PREDECESSOR's
   verify crate and types (one verify-core flavour per OpenVM version, e.g. a v2.0.2 copy with these two calls).
   Untested yet: the platform crate and the guest build on v2.0.2.
   A rehearsal: two images from different OpenVM tags (2.1.0 -> a 2.1.x or the next minor), a short chain on the old,
   handed over to the new, verified end to end; negative controls (a wrong pin, a tampered old proof, a journal that
   is not the old proof's).
