# Round 1, verifier-side fixes (R1-6 #1-#4; 27.11.2026)

## R1-6 #1: a key is never pinned to itself

Assessed: every CLI hashed the key file it was given and used that as the pin (`with_engine_key` on the bytes;
`corecheck::with_key(.., None)`; headers / hints / utreexo / zeroconf built `EngineKey { pin: sha256(file) }`). So a
forged key shipped next to a proof (a forged mvk, the genuine baseline copied in) passed every check under the image id
the reader typed. Paths: swiftsync-verify, zapp-assume, -deposit (core and header keys), -headers, -hints, -liveness,
-statement (draft and check), -utreexo, -zeroconf. The C ABIs and wasm frames already took the pin from the caller and
refuse a mismatch (their tests: swiftsync-verify-ffi test_engine.c "another pin", the wasm frame tests'
"key not the pinned one" in core, headers, hints, liveness, statement, utreexo): unchanged.

Fix: `swiftsync_verify::read_pinned_key(path, pin, what)` (appended at the END of lib.rs, so no wasm panic location
moves): refuses with no pin ("--vk needs its pin ...: the image id does not bind the key, so an unpinned key proves
nothing") or a pin that is not SHA-256(file). Every CLI takes `--vk-sha256 <64 hex>` (deposit also
`--headers-vk-sha256`) and reads its key(s) through it; assume_check.sh takes VK_SHA256. `corecheck::with_key` itself is
unchanged (its body is in four wasm modules); its doc now says `None` is for tests only (same line count, so the
modules' bytes do not move). Docs: every command example in docs/libzync/*_ZAPP.md, LIBZYNC.md, the module READMEs,
swiftsync-verify/README.md and engine/VERIFIER.md shows the pin, with one line that the image id alone adds no security.

New refusals (each: no pin, and a pin that is not the key's): swiftsync-verify test.sh (+3, incl. a changed key pinned
to its own hash, refused by OpenVM's check), vectors manifest (+2 cli_only cases), and the test scripts of assume,
deposit (+3, incl. the header key without its pin), headers, hints, liveness, statement, utreexo, zeroconf.

## R1-6 #2: engine/wasm-verify

The probe exported `verify` / `verify_proof` (the STARK under an UNPINNED key: "valid" for a forged key and proof) and
`image_id_hex` (from the forgeable key). Removed the two checks (and run.cjs, which called them); renamed the id
function `unpinned_image_id_hex` (display only, documented); only `verify_full` (with the pin) is a check. The module
doc's "compressed encoding" corrected to the plain encoding HZE1 carries. Not a published module (FEASIBILITY Q11).

## R1-6 #3, #4: zeroconf

The journal decoded through `swiftsync_verify::decode_journal` (its whole-words check first); a trusted core record
that names no image is refused ("the trusted <net> core image record names an image") instead of comparing a zero id.
Test: tests/engine.rs, a no-image record refused.

## Tests (laptop, 27.11.2026)

- swiftsync-verify test.sh 22/22; vectors 18/18 (native), 14/14 (wasm, test-vectors.mjs, now taking each case's pin).
- swiftsync-verify-ffi test.c all cases as expected; test_engine.c ENGINE_C_OK.
- every module's test.sh rc 0: assume, deposit, headers, hints, liveness, statement, utreexo, zeroconf (CLI 13 cases),
  common, corecheck (ZAPP_*_TESTS OK; LIVENESS_CLI 35 passed; ASSUME_C_ABI 6/6; LIVENESS_C_ABI 20/20; the wasm tests
  as before). Each test.sh rebuilt its wasm module in the pinned container and found its README's sha: no wasm module's
  bytes changed. The core module rebuilt: 1f6ebed9 (unchanged). So no re-pin, and no second-machine check is needed
  for this change.
- engine/wasm-verify cargo test passes; zapps/zeroconf cargo test passes.
