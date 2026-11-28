# Round 3: assessment of every finding (28.11.2026)

Six reports (r3_1 .. r3_6) at a9b78510. Summary: no forgery of the core statement or of any zAPP statement found.
OpenVM's circuits enforce what Hazync relies on (R3-1); the transaction rules match Core v28.0 against its real source
(R3-2); the coin log, BIP30 tree and trie hold (R3-3); hints / Utreexo (R3-4) and assume / deposit (R3-5) hold given
genuine ids and keys. The real defects are on the OPERATOR side, in pod / release code written 28.11 (R3-6 #1-#4): none
has run (no pod has booted with it, no release exists), and all must be fixed before any pod boot or release.

## Checked by me, to fix before any pod boot or release

| # | finding | checked | decision |
|---|---|---|---|
| R3-6 #1 | the engine pod boot prints SHA_BAD and continues (runs the downloaded host / hzengine); the controller accepts if the marker words appear anywhere in stdout, which the unverified binary could print; contributor keys are then copied | YES: sponsor_bot.py:1245 `sha256sum -c ... && echo SHA_OK \|\| echo SHA_BAD`, nothing stops; :1498-1500 substring test; a bad file is never refetched (:1242 `[ -s $f ] && break`) | the boot verifies every file against the controller's want.txt (from the locally verified signed manifest) BEFORE executing anything and exits non-zero on any mismatch (refetching a bad file up to 3 times); the controller matches the markers as exact, line-anchored lines, not substrings (the real guard is the one before: nothing unverified ever executes, so no unverified binary can print them); contributor keys only after acceptance (keep); the same in tip_engine's boot |
| R3-6 #2 | fold_rent's CPU hzengine is checked against a SHA256SUMS.txt fetched on the pod, no signature | to check in the fix | the controller passes the expected sha (from the locally verified manifest) as want.txt, as the sponsor boot does; nothing runs before it matches |
| R3-6 #3 | release-sign.yml loops over engine-release.tsv records, not over the key tarballs: a tarball with no record is signed unchecked; duplicate names overwrite; tar member names normalised with lstrip("./") | to check in the fix | iterate over EVERY hazync-keys-*.tar.gz in the release; each must have exactly one record; refuse duplicates; read tar members by their exact names (refuse ./, .., absolute, duplicates) |
| R3-6 #4 | every engine smoke calls verify-any without HAZYNC_ENGINE_VK: lz_coord.rs refuses every engine proof, so the smokes cannot pass; the tests' fake host answers RANGE-OK regardless | YES: lz_coord.rs:90-98; sponsor_bot.py:1267, run-workers.sh:226, tip_engine.py:127 | the boot ships the key file's pin from the locally verified release record and sets HAZYNC_ENGINE_VK (the extracted key file) + HAZYNC_ENGINE_VK_PIN; NO unpinned fallback; the fake host in every test checks both variables and refuses without them (so the test can fail) |

## To fix (not exploitable, or defence in depth)

| # | finding | decision |
|---|---|---|
| R3-4 M1 | hints / utreexo readers return OK for caller-named image ids (no built-in ids before the freeze; a caller id does not cap the verdict) | uniform rule in every reader: OK only for an image in the reader's built-in (published) records; a caller-named image gives at most DEMO, saying why (check swiftsync-verify / corecheck / every zAPP apply the same rule; the hints OK line, wasm and FFI say it too) |
| R3-4 L1 | hints takes the network from the proof; the lookups follow it; the OK line does not name it | --network (default mainnet) in hints lib / CLI / wasm / FFI, compared with the proof's; printed on the verdict |
| R3-4 L2 | no expected-format input for hints | expect_format input; FAIL on mismatch |
| R3-4 L3, I2 | the OK line does not say if undo was checked; a dead check | say it; remove the dead check |
| R3-4 I1 | tip step uses Stump::modify without verify | differential test (modify alone vs verify+modify on mutated proofs) |
| R3-5 L1, L2 | deposit doc implies min work alone makes OK unforgeable with --no-require-identity; ASSUME_ZAPPS.md still RISC Zero (profiles, trust root, line refs) | docs |
| R3-5 L3 | the C ABI treats non-NULL + length 0 as an empty input (FAILs) where wasm treats it as absent | header documents NULL = absent; parity test |
| R3-5 L4 | corecheck's caches grow per caller-supplied image / key | bound them (small fixed capacity, evict) |
| R3-5 L5 | a deposit can name a coinbase too young to spend | a coinbase deposit needs depth >= 100 (maturity) or FAILs "coinbase not mature"; documented |
| R3-5 L6 | two RISC Zero crash artifacts under zapps/assume/fuzz/artifacts | retire (git rm) with a note |
| R3-6 #5 | zeroconf wasm: `now` not bounded, the input frame leaks, a DEMO path unreachable, tests on the old image | free the frame; the glue supplies Date.now and documents it; drop or test the unreachable path; the new image's fixture when the regtest chain is proved (C8 agent, (B)) |
| R3-6 #7, #8 | pins are the signer's word until a rebuild re-derives vk.bin; windows-installer.yml checks hashes but not the manifest's signature | #7: reproduce/engine-ids.sh exports and compares vk.bin (4.1 work); #8: the installer job verifies SHA256SUMS.txt.asc with the release key before using it |
| R3-6 #9 | the app reads keys/image.json, not tied to the keys | the app checks image.json's key pin = SHA-256(vk.bin) |
| R3-3 L5 | COINLOG.md prints format-1 vectors; agg_lite.rs:18 says "SwiftSync total" | docs: format-2 vectors (both, labelled); the comment fixed on the same line |
| R3-2 L-1 | the 4 MB tx guard runs after Core has parsed the tx (comment says before) | move the length check before read_bytes (guest change: moves the core image; re-run the honest chunks) |
| R3-1 #1 | add_ne / double / div have unconstrained degenerate cases; Hazync's guards avoid them | write the invariants in msm_ovm.rs's header doc (same line count, or with the core image change above) and engine/guest/README; a fuzz test with repeated keys and repeated R points (batch-test / msm-test) |

## Noted, no change

R3-1 #2 (chip outputs byte-ranged only; every use canonicalises), #3 (output_len hint: only the prover's own run
fails), #4 (deferral input commitment chosen by the prover: bound by image id and public values); R3-3 L1-L4, L6
(refusals or stalls only; IV level sharing unreachable); R3-2 I-1..I-7 (sound; I-6's allocator-reuse startup check:
confirm it runs on the engine guest, with the guest change); R3-4 tip-mode Empty-sibling (liveness, shared with
Floresta).

## For round 4

def_hook_commit binding the verify-stark circuit's full key incl. constants (R3-1, medium confidence); the OpenVM
memory argument and program buses (assumed from the v2.0.0 audit); the speculative chunk's height-dependent rules for
the block after the tip (R3-6); the engine assembly (edits.py, coinlog_ovm.py) against the RISC Zero guest source
(R3-3, R3-5); the C++ helpers read only at call sites (hz_tx_rules_c, hz_indicated_height, is_bip30_unspendable, the
caches).
