# OpenVM's verifier in WebAssembly (FEASIBILITY Q11, 25.11.2026)

A probe, not a product: can a browser check an engine proof?

- `openvm-abi-stable.patch`: OpenVM v2.1.0 (05f3b73e) with `abi_stable` removed from openvm-circuit. It was a hard
  dependency used for one type (`RVec`, in the metered executor's memory context), and it pulls in `libloading`
  (native dynamic loading), which cannot build for wasm32. `RVec` becomes `Vec` there; nothing in the verifier uses it.
- `src/lib.rs`: `verify_full(vk, vk_sha256, proof, journal, image_id)` via wasm-bindgen (below), the only check it
  exports. ⛔ 27.11.2026 (review R1-6 #2): the STARK-only `verify` / `verify_proof` of 25.11 took an UNPINNED key and
  answered "valid" for a forged key and proof; they are removed (and run.cjs, which called them). `image_id_hex` is
  `unpinned_image_id_hex`, for display only. The proof is now OpenVM's plain encoding (what the HZE1 file carries).
- Inputs from `hzengine wasm-inputs --dir <keys> --proof <proof> --out <dir>` (vk.bin, proof.zst, proof_bad.zst with
  one encoded byte flipped).

Build: apply the patch to a copy of OpenVM v2.1.0 at ../../../openvm-wasm, then
`RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo build --release --target wasm32-unknown-unknown` and
`wasm-bindgen --target nodejs --out-dir pkg target/wasm32-unknown-unknown/release/wasmv.wasm` (wasm-bindgen-cli 0.2.130).

**Measured** (block 130,000's aggregate proof from the CPU segpar test, which `hzengine verify` also accepts; laptop,
Node 24): wasm 708,710 bytes; vk 96,425 bytes; proof 229,466 bytes zstd (273,107 encoded); **valid proof accepted in
24-45 ms, the flipped-byte copy rejected (0)**; native verifier on the same bytes 0.060 s (build box).

(25.11, historical) `verify_proof` checked the STARK against the key and baseline (OpenVM's verify_vm_stark_proof_pvs also checks the user
public values' Merkle proof against the proven final memory, the program commitment, initial memory and exit code).

**`verify_full(vk, proof, journal, image_id)`** (26.11.2026) adds what a page needs to trust a result:
- the image: SHA-256(exe ‖ vm) computed from the vk's baseline (engine.rs `image_of`: exe = the baseline's
  app_exe_commit, vm = OpenVM's poseidon2_hash_slice over the app/leaf/internal-for-leaf vk commitments, re-implemented
  over stark-sdk's poseidon2_perm) must equal the published image id;
  ⛔ **CORRECTED 26.11.2026 (evening review): the vk file MUST be trusted (pinned).** OpenVM verifies the STARK under
  `vk.mvk` and only then compares the proof's public values with the baseline; nothing ties `mvk` to the baseline or the
  image id, and a forged `mvk` (a circuit that outputs the expected commitments) would pass. `verify_full` now takes
  the key's SHA-256 pin (code 7 if it differs); a page carries the pin as a constant. Nothing deployed relied on this.
- the journal: the proof's 32 user public values (one byte each) must equal SHA-256(journal).
Codes: 1 valid, 0 STARK invalid, 2 bad vk, 3 proof does not decode, 4 journal mismatch, 5 image mismatch, 6 id not 32 B,
7 not the pinned key (`verify_full(vk, vk_sha256, proof, journal, image_id)` since 26.11 evening).

**Measured** on the same real proof (block 130,000's aggregate, keys-hz24, its block.journal and image.json):
- native test (`WASMV_INPUTS=<dir> cargo test --release`): image_of(vk) == image.json, and valid 1, one journal bit
  flipped 4, another image id 5, short id 6, flipped proof byte 0, truncated vk 2;
- wasm (736,681 bytes) in Node 24 (`node run_full.cjs <inputs> $PWD/pkg`): the same codes; valid in 25-39 ms;
- wasm in a browser, Chromium headless shell 156 (`browser.html` served with `in/` and `pkg-web/`, read with
  `--virtual-time-budget --dump-dom`): the same codes, image id from the vk equal to image.json's. Timings there are
  not real (virtual time freezes the page clock); Node runs the same V8, so its times stand for a desktop browser.
- Not yet: a phone.

**26.11.2026: the checks moved to engine/verify-core** (crate `hazync-engine-verify`), shared with swiftsync-verify
(engine/VERIFIER.md step 2); this crate is now its wasm-bindgen face, OpenVM [patch]ed with the abi-stable patch here.
Re-measured after the move: native test (`WASMV_INPUTS=$PWD/in cargo test --release`) passes; wasm 738,247 bytes; Node
24 `run_full.cjs`: valid 1 in 38.5 ms, journal bit 4, other image 5, flipped proof byte 0, ALL AS EXPECTED. verify-core
itself builds and passes its test (tests/data: the same proof, committed) on OpenVM v2.1.0 UNPATCHED (native).

**Source of the patched OpenVM (26.11.2026 late):** `engine/scripts/openvm_src.sh <dir> <patches>` clones OpenVM and
stark-backend v2.1.0 from their repos, refuses unless HEAD is the commit our lockfiles pin (05f3b73e / a6b4761a), and
applies the patches. Its output with this crate's patch (plus the handover's two) is file-for-file the tree this crate
was built and tested on (`diff -rq`: no file differs; only test-vector submodules a shallow clone leaves out). A
shipped build stages that output, never a hand-made copy.
