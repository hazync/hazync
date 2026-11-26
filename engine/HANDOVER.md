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
     image id commits to exactly that key; the predecessor's image id (`image`) must equal image_id_of(that key)
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
3. A rehearsal: two images from different OpenVM tags (2.1.0 -> a 2.1.x or the next minor), a short chain on the old,
   handed over to the new, verified end to end; negative controls (a wrong pin, a tampered old proof, a journal that
   is not the old proof's).
