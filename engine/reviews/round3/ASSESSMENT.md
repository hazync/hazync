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

## Status: operator-side fixes (28.11.2026)

Each with a test that fails on the code before the fix (6a10ef78) and passes after; the controls still fail.

| # | state |
|---|---|
| R3-6 #1 | FIXED (sponsor_bot.py, both boots; tip_engine boots through it): every downloaded file is checked against want.txt (built only after the manifest's signature verified locally, as before) BEFORE anything runs; a mismatch is fetched again (3 tries), then the boot exits 3 with nothing downloaded run; the controller matches SHA_OK / KEYS_OK / CUDART_OK / GPU_OK / MID=<id> as whole lines (boot_accepted). test_sponsor_engine.py: a tampered host never runs and stops the boot; a stale bad file is refetched; markers inside other text are refused; on the old code 13 checks fail |
| R3-6 #2 | FIXED (fold_rent.py, both CPU fetches): the expected sha comes from the controller's verified manifest (SshRunner.manifest_sha), no SHA256SUMS.txt fetched on the pod; refetch, then stop before the binary runs; no sha, no script. fold_rent selftest runs the fetch on a fake pod: a tampered binary is never run |
| R3-6 #3 | FIXED (release-sign.yml): records read strictly (four fields, hex, a name once, also checked before the core id is read); the key tarballs drive the check (every hazync-keys-*.tar.gz needs exactly one record, every record its tarball); members must be exactly the seven key files, each once, regular, plain names. test-release-sign.sh: 5 new cases (no record, a name twice, ./ names, a second vk.bin, ../vk.bin), all refused; the old workflow fails all 5; the control still fails |
| R3-6 #4 | FIXED: the engine boot pins the core key from the release itself (SshRunner.engine_key_pin: the key tarball checked against the verified manifest, its single vk.bin hashed; an operator SPONSOR_BOT_ENGINE_VK_PIN must agree); the pod refuses another vk.bin (KEYS_BAD, exit 4) and engine.env carries HAZYNC_ENGINE_VK + _PIN, so the smoke and tip_engine's per-block check run under the pinned key; tip_engine refuses to rent when the runner's pin is not HAZYNC_ENGINE_VK_PIN; run-workers.sh requires HAZYNC_ENGINE_VK_PIN (the key file defaults to $HAZYNC_ENGINE_KEYS/vk.bin) and checks it. No unpinned fallback anywhere; the fake hosts in test_sponsor_engine.py and test_tip_engine.py refuse without both (the tip test's fake host had the unpinned fallback that hid this) |
| R3-6 #8 | FIXED (windows-installer.yml): with a manifest on the release, SHA256SUMS.txt.asc must be there and verify under the maintainer key pinned in tools/win-gui/appupdate.py (new verify_manifest.py / appupdate.check_manifest: the signature AND the signed tag line) before any hash is trusted; test_appupdate.py adds 6 checks (incl. the real v0.26.1 manifest passing and a changed byte refused). The first signing (no manifest yet) is unchanged |
| R3-6 #9 | FIXED (tools/win-gui/supervisor.py): the image the app compares is the one the KEY commits to (host method-id under HAZYNC_ENGINE_VK + _PIN), and image.json must name the same image (else fatal); CANONICAL_ENGINE_VK_PIN (empty until the Stage 4 freeze) then refuses any other key. test_engine.py: a mixed key folder and a wrong published pin are fatal |
| R3-5 L4 | FIXED (zapps/corecheck): both caches bounded (64 images, 16 (image, key) pairs); past the cap a new entry is an error (a FAIL), never leaked; cached entries still answer; the pin is still checked first. tests/cache_bound.rs. ⚠ This changes corecheck's code, so the wasm modules built on it (assume, deposit, liveness, statement, zeroconf) change bytes: re-pin once, with the round-3 reader fixes |

Also fixed on the way: test_sponsor_engine.py's control exited 0 when it failed as it must, which CI's control step
(`if ... --control; then exit 1`) would have turned red; it now exits non-zero on any failure, like the others.

## Status: guest-side fixes (28.11.2026, 5cb6724e; fixes_guest.md)

| # | state |
|---|---|
| R3-2 L-1 | FIXED (main.rs chunk loop): a transaction over 4,000,000 bytes is refused right after its length is read, before its bytes are read or any of Core's code sees them (log `core-reject: bad-blk-length`, then the run stops: no proof). The only caller of ChunkCtx::record; its own guard kept, its comment now true. The zeroconf guest reads no tx bytes itself (it verifies a core chunk proof) |
| R3-3 L5 (comment) | FIXED: agg_lite.rs's header says the coins are bound by the coin log |
| R3-1 #1 | FIXED: the INVARIANTS block in msm_ovm.rs, the div site in batch_ecdsa_ovm.rs, engine/guest/README.md. msm-test: against RustCrypto's k256 (an independent implementation), 710 cases, all equal; a counting copy (build.rs, markers asserted unique) shows all 10 guarded branches reached. Controls: equal-x test removed fails; a = -p test removed fails on the comparison |
| R3-2 I-6 | CONFIRMED, no change: the engine guest's default features include libzync, so c_heap::startup_check() runs (main.rs:2404); it links OpenVM's default bump allocator (openvm features ["std"] only, not heap-embedded-alloc); a reusing-allocator build (keys-hz5) was refused by this check (engine/results/profile_chunk) |

New core image: **755f5c5af4688606ac17e516161d870077ef40fdfe39f104c79f253845119c77** (exe 0ee7ff3e..., vm 00e00077...
unchanged, vmexe c0301de5...), the same from reproduce/engine-ids.sh (laptop container) and the box's canonical layout.
Honest chunks on it: 27 of 27 byte-identical to the previous image's and to RISC Zero's (r3_run_chunks.out).

## Status: reader-side fixes (28.11.2026, 363a51ca; wasm pins 4d71f3e9 and the next commit)

| # | state |
|---|---|
| R3-4 M1 | CHANGED FROM THE DECISION ABOVE, on assessment: NOT a DEMO cap. Every reader documents a caller-named image id as "the image you rebuilt" (§5 step 9: swiftsync-verify/README.md, main.rs, every zAPP CLI's usage) and gives OK on it by design; a blanket DEMO cap would remove that trust root and, before the freeze (no image published), make every reader DEMO and force a test-only bypass into the shipped CLIs to keep the OK tests (deposit 40/40, headers whole chain). Instead the uniform rule the reviewer gave as the alternative: every report says when an image is the caller's (swiftsync_verify::Image::is_published, caller_image_note): the core verifier (so corecheck, liveness, statement, assume, deposit through it), hints, headers, utreexo (core image too), zeroconf (both images); the hints CLI's OK line and C ABI message say it. Whether a caller-named id should cap at DEMO AFTER the freeze, when published records exist, is a freeze decision (the user's). Tests: swiftsync-verify tests/engine.rs and hints tests/engine.rs, test.sh |
| R3-4 L1, L2 | FIXED: hints --network (default mainnet; a mainnet proof at a signet node FAILs) and --format bip/crate (the other format FAILs; not given: at most DEMO, said) in lib (HintsAsk), CLI, wasm (two frame chunks, glue `network` / `format`), C ABI (`network`, `format` arguments; header). Tests in all four, each failing on the old code |
| R3-4 L3, I2 | FIXED: no undo file is said in the report and the verdict line; the dead lineage check removed |
| R3-4 I1 | TESTED, no finding: zapps/utreexo/tip-step/tests/modify_vs_verify.rs (in test.sh): 60 random accumulators, 9,659 mutated (proof, deletions) pairs; Stump::modify alone accepted none that Stump::verify refuses; 657 both accept (reordered targets / leaves), each with the honest roots when the same leaves are deleted |
| R3-5 L1, L2 | FIXED (docs): DEPOSIT_ZAPP.md says what --no-require-identity leaves open (a min_depth fork above the minimum-work height); ASSUME_ZAPPS.md on the engine (profiles 0xf1 / 0xf5, the key pin and OpenVM in the trust row, line numbers re-checked in today's guest source, edit 10's x) |
| R3-5 L3 | FIXED: zapp_assume.h says NULL = absent, a non-NULL empty input is given and FAILs; test_assume.c tests both |
| R3-5 L5 | FIXED: a coinbase deposit needs COINBASE_MATURITY (100) confirmations, else FAIL "coinbase not mature" (lib, so CLI, wasm, FFI); tests: 31 confirmations refused, 3,862 accepted, a non-coinbase not held to it; DEPOSIT_ZAPP.md §3 |
| R3-5 L6 | FIXED: the two RISC Zero receipt artifacts git rm'd, zapps/assume/fuzz/README.md says why |
| R3-6 #5 | FIXED: the zeroconf module takes and frees its input frame, free_result frees reports (the glue calls it); the Node test makes 40 calls and checks memory does not grow by them (the old module grows by 40 proofs); `now` is the caller's clock, documented in the glue and README. The anchor input stays while the fixture is the earlier image's (it is tested there); it goes with the fixture on image 132969d8, where it can only FAIL |
| R3-3 L5 | FIXED (COINLOG.md): format-2 vectors and the engine chain's roots at 200 (from the verifier's manifest), format 1 kept and labelled |
| wasm | every module re-pinned on the laptop in a clean worktree, the published image: core 806c9531, hints 83aa3dce, headers 6eb5f99f, utreexo 8c4d818b (at 363a51ca), assume b5c04bbe, deposit 3c1f468d, liveness 15684135, statement 6b912526, zeroconf 174bf4fc (at 34adaa2d, after corecheck's cache bound). Every zapps/*/test.sh, swiftsync-verify, the core vectors on CLI / C ABI / wasm (18/18, 14/14), every C ABI test pass on the new pins. The second-machine check is PENDING |
