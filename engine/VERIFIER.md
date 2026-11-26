# Verifying engine proofs: swiftsync-verify on OpenVM (roadmap 2.4, design, 26.11.2026)

Goal: the released verifiers (swiftsync-verify's CLI, its C ABI, its WASM, and corecheck) accept a proof of the
engine chain with the same checks and the same verdicts they give a RISC Zero receipt today, and keep accepting the
frozen RISC Zero chain's receipts.

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

## Telling the two apart

A RISC Zero receipt file starts with bincode's InnerReceipt variant (u32 LE 0-3). The engine proof is a new container:
a 4-byte magic (`HZE1`), then the zstd of OpenVM's codec encoding (what `hzengine wasm-inputs` writes as proof.zst).
The magic is checked first, so neither decoder is ever handed the other's bytes.

## Recommendation (needs the user's OK: it decides the release shape)

**One verifier for both provers.** The frozen RISC Zero chain stays checkable with the same tool, a user never has to
know which prover a file came from, and the statement checks (most of the code, and all of its review history) stay
in one place. Cost: the WASM grows by OpenVM's verifier, measured at ~0.7 MB (wasm-verify: 736,681 bytes standalone).
The alternative, a separate engine verifier, duplicates steps 3-12.

## Work, in order

1. Move today's steps 1-3 into the RISC Zero front; the existing test suite must pass unchanged (no behaviour change).
   **Done 26.11** (`risc0_front`): the crate's tests 33 passed / 1 ignored, before and after; the CLI built from the old and
   the new lib.rs gives the same report, line for line (only the verifier's own clock differs, by the second), on block
   700,000's real receipt (it verifies against image 46666b6f... and runs the statement checks), block 976,000's, the
   hints vector, and a file that is not a receipt.
2. A shared crate for the engine front (`engine/verify-core`: wasm-verify's `verify_full` and `image_of`, no
   wasm-bindgen), used by wasm-verify and swiftsync-verify. It needs the abi-stable patch for wasm32 (OpenVM 2.1.0).
3. The engine front in swiftsync-verify behind the magic; engine images in the registry (the image's vk embedded).
4. Tests: the engine chain 1..200's proof of block 200 accepted with the same Report as its RISC Zero twin (apart
   from the image lines); every negative the RISC Zero suite has, ported (flipped proof byte, wrong image, journal
   edit, another image's vk); the fact checks (swiftsync-verify's block / coin / trie / tx facts) on the engine chain.
5. The C ABI and WASM builds; corecheck; the web page (Stage 3.5).
6. Review rounds (the user's rule) before any release.
