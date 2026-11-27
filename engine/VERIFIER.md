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

## The proof file holds OpenVM's PLAIN encoding (26.11.2026 late)

`HZE1 | u32 LE length | proof | journal`, the proof as OpenVM's codec writes it (`decode_from_bytes_using_vk`), not the
zstd-compressed form: zstd is a C library, and without it no verifier build compiles C (the wasm engine graph has no
zstd / cc / cmake: checked with cargo tree), so the wasm build needs no C toolchain image. Cost: ~19% larger files
(block 200: 273,107 bytes vs 230,191). The test data was regenerated from the same proofs; every engine suite passes
(verify-core 1, swiftsync-verify 39, corecheck 7, liveness 18, assume 32, deposit 25, statement 21, headers 18,
wasm-verify native 1 and Node, the C ABI's test_engine.c, the engine wasm module's frame test: 1,406,503 bytes now).

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
   **The key in the image RECORD, 26.11 late:** `Image.engine_key: Option<EngineVk>` (bytes + pin); plain `verify`
   checks an engine file under the record's key (refused, with the reason, when the record has none), so corecheck and
   every zAPP surface take engine proofs from the record alone: a published engine image embeds its key; a reader's
   rebuild uses `Image::with_engine_key(file)`. corecheck (`--features engine`, tests/engine.rs): block 200's engine
   core proof with `expect 200:<tip>` all checks ok, tip_checked; another expected block FAIL; the RISC Zero record
   FAIL ("no engine verifying key"). All dependents compile (deposit, liveness, statement, assume, common, ffi, wasm).
   **C ABI, 26.11 late:** `swiftsync_verify_engine_facts(proof, ..., vk, vk_len, vk_pin, facts, expect, ...)`
   (swiftsync-verify-ffi `--features engine`; header documents that the pin must be the PUBLISHED one). test_engine.c
   on the engine chain's block 200: DEMO with profile 0xf1 and 209 coins; with block 9's coinbase spent-fact DEMO; FAIL
   for: the fact with another spender, another pin, a journal bit, no key, an engine proof given to swiftsync_verify.
   The default library: test.c's output identical to test.out (17 lines).
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
   **The real crate, 26.11 late:** swiftsync-verify-wasm `--features engine`; frame network bit 0x40 = an engine proof
   file, followed by the key and its 32-byte pin (a constant of the page). Built (scratch copy with the [patch]es and
   the cfg, OpenVM from openvm_src.sh): 1,503,705 bytes, `test-engine-frame.mjs` in Node: block 200 verdict demo, no
   failed check, 71 ms; refused: a journal bit, a non-pinned key, another pin, an engine file without bit 0x40.
   The DEFAULT module (no engine) is unchanged in behaviour: test-vectors.mjs output identical to test-vectors.out (15
   verdicts), test-facts-frame.mjs FACTS_FRAME_OK; its bytes do change (shared code moved), so the next release pins a
   new sha. Cargo.lock gains the engine graph as optional edges only (every old package, version and checksum kept).
   Left for the reproducible build (release tooling, the user's call before it ships): the build container has no git
   and its image is published by digest, so make the patched tree on the HOST with openvm_src.sh, pin a manifest
   (SHA-256 per file) in the repo, verify it in the container, stage it at a fixed path, pass the [patch] tables with
   `cargo --config`, and a separate locked lockfile for the engine graph.
6. Review rounds (the user's rule) before any release.

## The verifier-side zAPPs on engine proofs (roadmap 2.5; 26.11.2026 late)

Each with `--features engine`, a test (tests/engine.rs) on the engine chain's proof of mainnet 1..200, its image record
carrying the key; inputs made from the ENGINE chain where they depend on the node format:

| zAPP | test | result |
|---|---|---|
| corecheck | verify_core + expect 200:<tip> | all checks ok, tip matched; wrong tip and keyless record FAIL |
| liveness | 5 claims (block 9 coin spent at 170; Hal's coin unspent with its value; change spent at 181; change unspent after 170, opened through the forest; created/coinbase pinned), proofs from the engine history (`zapp-liveness prove`) | all hold; 7 wrong questions and a format-1 proof refused |
| assume | Core v31.1's real UTXO snapshot at 200; assumevalid | 204 coins, Core's hash_serialized_3; the 199 snapshot and a changed byte refused; block 200 named |
| deposit | block 170's Satoshi-to-Hal tx, core proof alone (bundle from the engine proof + engine history) | every check ok (in the fully validated chain, output 0 = 10 BTC, 31 confirmations); another amount, a deeper ask, and a bundle from the RISC Zero run's history refused |
| headers | the header zAPP's engine proof of headers 0..4,031 (`verify_headers_with` + key) | accepted, tip = block 4,031's hash, chainwork 0xfc00fc00fc0 (4,032 headers); refused: another tip, a journal bit, another pin, another image, a repeated run, an engine file without the key, a non-engine file |
| deposit, header-bound | the core tip 200 placed in the engine header chain 0..4,031 (MMR path, 11 hashes) | every check ok: 3,862 confirmations (31 fully validated, 3,831 proof of work only); one more refused; the header run without its key refused |
| utreexo (27.11) | the utreexo zAPP's own engine proofs: the base (roots after block 100, core tip 200) and a tip step 100 -> 150 on it (`verify_utreexo_with` / `verify_utreexo_tip_with` + the utreexo image's key; CLI `--vk`) | both accepted, DEMO (below minimum work), blocks 100 / 150 / 200 matched; 12 refusals (another block or core tip, a journal bit, a proof byte, another image id, a pin not the key's, another image's key, a tip proof as a base, another core image, signet, the engine file read as a RISC Zero receipt, a tip step under another base) |
| hints (27.11) | the hints zAPP's own engine proof for H = 200 with the host's hints and undo files (`verify_hints_with` + key; CLI `--vk`) | accepted, DEMO, block 200 matched: 209 outputs indexed, 204 unspent, 10,000 BTC; refused: a hints byte, an undo byte, another block 200, a journal bit, a proof byte, another image id, a pin not the key's, another core image, the file without a key |
| statement | two fresh keys' zero balances (absence proofs) on the engine chain's trie at 200 (native prep; its state = the engine-proven one but self_id), BIP-322 signed | holds; a claimed 1 sat refused; the format-1 trie file refused by the store (OldRoot) |

| core facts | trie facts on the engine chain's trie at 200 (Hal's coin unspent with its origin, its script's balance and complete history, an absent outpoint) and Hal's coin journey (zync-path journey) | all hold; a wrong claim on each trie fact refused |

Each zAPP's default suite unchanged (deposit 23, statement 20, corecheck 6, headers 17 passed; swiftsync-verify 33 default,
39 with `engine`; utreexo 3 + 10, and 6 with `tip`; hints 2).

With a key, utreexo and hints trust the named core image as an ENGINE build (`Image::with_node2`, profile bit 7): an engine zAPP
proof can only rest on an engine core proof, because its child is verified in the same VM. Its wasm module has no
engine build yet (like the other zAPP modules: only swiftsync-verify-wasm has one).

**Engine image RECORDS wait for the Stage 4 freeze, on purpose:** `Image::by_network` takes the LAST record of a network
as its current image, so a record for a development image would change what every default caller checks against. At
the freeze: the record (id, profile with PROFILE_NODE2, the new chain's anchor, checkpoint) with its key embedded
(include_bytes of the key file, its pin beside it, a unit test that the two agree and that image_id_of(key) = the id).

Left: the zAPP CLIs / C ABIs / WASM taking a key for a
reader's own rebuild (published records will carry it), trie / tx / journey facts in the core verifier.

## The reproducible engine WASM build (26.11.2026 late)

`swiftsync-verify-wasm/build.sh --engine` -> `swiftsync_verify_wasm_engine.wasm`, in the PUBLISHED build image
(ghcr.io/hazync/hazync-wasm-build@sha256:d46e4164..., unchanged; no CI run): on the host, engine/scripts/openvm_src.sh
fetches OpenVM + stark-backend v2.1.0 at the pinned commits and applies the three patches under the crate's target/; the
build refuses unless engine/scripts/tree_digest.sh gives the pinned 38eccd09... (2,123 files), and the CONTAINER
recomputes that digest before staging the tree; then `Cargo.engine.lock` (generated by the container's own cargo
1.99.0: the default lock plus the patched path crates, minus zstd / zstd-safe / zstd-sys / pkg-config), the [patch]
tables by `cargo --config`, the refusing random source's cfg.
**Measured:** 1,405,695 bytes, sha256 2b1b8a33...; a SECOND build with OpenVM re-fetched and re-patched from nothing
gave the SAME sha256; the module passes test-engine-frame.mjs in Node (block 200 demo, the four refusals).

Found on the way, fixed: cargo loads EVERY path dependency, optional or not, so the default build failed once
swiftsync-verify named engine/verify-core (optional): build.sh now stages it always. The default module: 1,241,166
bytes (1,240,367 before the verifier port's code; no OpenVM crate in its graph). Transitional cost: the default build
also clones OpenVM's and stark-backend's git repositories (cargo resolves the optional git dependency's manifest);
gone when the engine path becomes the only one (then the pinned tree is staged for every build).

## When the RISC Zero front goes: at the cutover, not before

Nothing blocks the removal any more (the engine wasm build is reproducible). But until the cutover (roadmap 6.3) the
published verifiers must keep checking the chain that is LIVE, the RISC Zero one; the engine path stays opt-in
(`--features engine`, `build.sh --engine`) on this branch. At the cutover, in one change: `engine` becomes the only
front (the default), the RISC Zero front, its receipt kinds, dev-mode check, Groth16 and earlier-image logic go, the
engine image records go in with their embedded keys, the zAPP fixtures move to engine proofs, the published module
shas are renewed (README, the site, deploy.sh), and the old chain stays checkable with the last RISC Zero release.
