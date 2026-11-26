# Verifying engine proofs: swiftsync-verify on OpenVM (roadmap 2.4, design, 26.11.2026)

Goal: the released verifiers (swiftsync-verify's CLI, its C ABI, its WASM, and corecheck) accept a proof of the
engine chain with the same checks and the same verdicts they give a RISC Zero receipt today.

**Decided (user, 26.11.2026): the new chain is OpenVM only, re-proved from genesis; no RISC Zero proofs in it.** So the
new verifier checks ENGINE proofs only. The frozen RISC Zero chain stays checkable with the verifier versions already
released (roadmap 6.4); the new release does not carry RISC Zero's verifier.

## What changes and what does not

`swiftsync_verify::verify(&Inputs) -> Report` today does, in order:

1. decode the receipt (bincode `risc0_zkvm::Receipt`; kind checked first: succinct or Groth16, never Fake);
2. `receipt.verify(METHOD_ID)` (steps 1 and 8);
3. decode the journal as `ChainState` (step 2) and set the node format from its profile;
4. everything else (steps 3-12): self_id, profile, lineage and handovers, anchor, work, time, the verdict.

Only 1-3 know which prover made the proof. Steps 3-12 read the `ChainState` and the image record. So the port is:

- **A prover-specific front**: `fn proof_front(bytes, image) -> Result<(journal bytes, checks), checks>`, one per prover.
  The RISC Zero front is today's code moved, unchanged. The engine front is `engine/wasm-verify`'s `verify_full`
  (tested 26.11 natively, in Node and in a browser): the STARK against the image's verifying key, the image id
  recomputed from that key (SHA-256(exe ‖ vm)) and compared with the published id, and the proof's 32 public values
  equal to SHA-256(journal).
- **The statement checks unchanged**, fed the journal bytes. `cs.self_id == the image id` already ties the journal
  to the image; on the engine the self id is the engine image id (`--self-id`, results/*).
- **The image registry** (`Image`, pinned ids, handover list) gains the prover: an engine image is its 32-byte id plus
  its verifying key (agg.vk + baseline, ~96 KB bitcode; embedded like the RISC Zero ids are today, or shipped as a
  file whose hash is pinned). Handovers stay a list of image ids; an id says which prover it is.

## The verifying key must be PINNED (review finding, 26.11.2026 evening)

OpenVM's host verifier checks the STARK under `vk.mvk` (the internal-recursive layer's key) and then compares the
proof's public values (program commitment, initial memory, the app / leaf / internal-for-leaf key commitments) with
the baseline. Nothing ties `mvk` to the baseline or to the image id: those public values are whatever the circuit `mvk`
describes, so a forged `mvk` (a trivial circuit outputting the expected values) passes every check. The image id
(SHA-256(exe || vm)) therefore does NOT authenticate a key file. Every engine image record pins **SHA-256 of its key
file** (agg.vk + baseline, bitcode), checked before the key is decoded (verify-core `verify_full_bytes`, code 7). The
key may then travel with the proof or be shipped beside the verifier: either way only the pinned bytes pass. (In the
guest, child proofs go through the VM's own deferral circuit, which the image itself fixes: not affected.)

## The proof file

The engine proof is a new container: a 4-byte magic (`HZE1`), then the zstd of OpenVM's codec encoding (what
`hzengine wasm-inputs` writes as proof.zst), then the journal. The magic is checked first, so a RISC Zero receipt (or
anything else) is refused with a clear line ("not an engine proof: check the old chain with release <x>") instead of
being handed to a decoder.

(Superseded: the recommendation of one verifier for BOTH provers. Not needed: the new chain has no RISC Zero proofs.)

## Work, in order

1. Move today's steps 1-3 into the RISC Zero front; the existing test suite must pass unchanged (no behaviour change).
   **Done 26.11** (`risc0_front`): the crate's tests 33 passed / 1 ignored, before and after; the CLI built from the old and
   the new lib.rs gives the same report, line for line (only the verifier's own clock differs, by the second), on block
   700,000's real receipt (it verifies against image 46666b6f... and runs the statement checks), block 976,000's, the
   hints vector, and a file that is not a receipt.
2. A shared crate for the engine front (`engine/verify-core`: wasm-verify's `verify_full` and `image_of`, no
   wasm-bindgen), used by wasm-verify and swiftsync-verify. It needs the abi-stable patch for wasm32 (OpenVM 2.1.0).
   **Done 26.11**: `hazync-engine-verify` (typed `Refusal`, wasm-verify's codes kept); its test on a real proof passes
   natively on OpenVM v2.1.0 as released; wasm-verify rebuilt on it passes natively and in Node (engine/wasm-verify/README.md).
   The abi-stable patch is still applied to an OpenVM copy OUTSIDE the repo: a release build must apply it from the
   repo (as engine/handover/assemble.sh does its patches), or get it upstream (roadmap 4.3).
   **26.11 late:** `engine/scripts/openvm_src.sh` makes that tree from the pinned commits (fetched and checked, then
   patched); next, swiftsync-verify-wasm/build.sh stages its output at a fixed path like its other path dependencies.
3. The engine front REPLACES the RISC Zero front in swiftsync-verify (behind the magic); engine images in the
   registry (the image's vk embedded, or a file whose hash is pinned); the RISC Zero-only parts (receipt kinds, dev
   mode, Groth16, `earlier_image_named_by`) leave with it. The journal stays RISC Zero's serde word format (the
   engine guest writes the same bytes), so only its decoder is kept.
   **Started 26.11 (late):** `verify_engine(&Inputs, &EngineKey)` behind the `engine` feature (off by default until the
   wasm build carries OpenVM's patch); steps 2-12 moved into `statement_checks`, shared by both fronts; `EngineKey` =
   the key file's bytes + its pin. The proof file (HZE1) comes from `hzengine wasm-inputs` (proof.hze, vk.sha256).
   Registry records for engine images and the CLI / C ABI / WASM entry points: to do (and then the RISC Zero front goes).
4. Tests: the engine chain 1..200's proof of block 200 accepted, its Report the same as the RISC Zero chain's for
   the same block (apart from the image lines); every negative the RISC Zero suite has, ported (flipped proof byte, wrong image, journal
   edit, another image's vk); the fact checks (swiftsync-verify's block / coin / trie / tx facts) on the engine chain.
   **First part done 26.11 (late)** (swiftsync-verify/tests/engine.rs, `--features engine`): the engine chain's proof of
   block 200 (keys-ch) passes EVERY check (proof, ChainState v2 with self_id, profile 0xf1 incl. node format 2, time,
   block forest, lineage, genesis anchor, coin log; verdict Demo only for low work and the checkpoint); refused: a
   journal bit, a proof byte, a non-pinned key, another pin, another image's record, a RISC Zero receipt's bytes, a
   truncated file; the RISC Zero front refuses an engine file. Suites: default 33 passed, `engine` 35, 1 ignored each;
   the CLI's reports on real RISC Zero receipts identical before and after (same --now). Coin facts (zync-path on the
   engine chain's history.bin) on the verified engine state: block 1's coinbase UNSPENT at 200 (with its preimage), block
   9's coinbase SPENT at 170 by tx 1 input 0; refused: a path node, a wrong preimage, no preimage, spent claimed
   unspent. Trie, tx and journey facts: to do (they need the engine chain's trie dump / tx facts from the host).
5. The C ABI and WASM builds; corecheck; the web page (Stage 3.5).
   **WASM probe 26.11 late** (a scratch copy of swiftsync-verify-wasm, `--features engine` on swiftsync-verify):
   compiles for wasm32-unknown-unknown as a RAW module (no JS glue) given three things: (a) OpenVM's abi-stable patch;
   (b) stark-backend's nologging patch and EVERY stark-backend crate [patch]ed to that one copy: without it stark-sdk's
   metrics stack pulls quanta -> web-sys -> wasm-bindgen and the module imports `__wbindgen_*` placeholders; (c) a
   random source that REFUSES, for both getrandom versions OpenVM's VM crate links through rand (0.2 `custom`
   feature + register_custom_getrandom!, 0.3 `--cfg getrandom_backend="custom"` + `__getrandom_v03_custom`):
   verification never draws randomness (Fiat-Shamir), so any call failing is the safe answer. Result: **1,264,189
   bytes, imports: none**. Not yet run on a proof (needs the frame's key + pin: network code bit 0x40, next).
   For the reproducible build: build.sh must make the patched sources (engine/scripts/openvm_src.sh) and stage them at
   a fixed path, and add the cfg to its RUSTFLAGS.
6. Review rounds (the user's rule) before any release.
