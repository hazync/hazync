# C8: the RISC Zero prover out of the build (survey and plan, 28.11.2026)

engine/CUTOVER.md row C8. The user cleared the cutover on 27.11.2026. The board (3.2, 855a8d7a) and the tip and fold
tools (3.1, f5207731) drive the engine. This file is the survey, made before anything was removed, and the order of
the work. The frozen RISC Zero chain is not this branch's concern: its proofs stay checkable with the verifiers v0.27.1
released (ROADMAP 6.4).

## Survey: where RISC Zero is still built or used

Counted on engine/openvm at f5207731 (`/usr/bin/grep -c`).

| where | what | lines / refs | used by the engine path? |
|---|---|---|---|
| prover/Cargo.toml | the workspace: host, methods, zyncapps/methods, zyncapps/host; [patch] of six vendored crates (risc0-zkvm, risc0-circuit-rv32im-sys, -keccak, -keccak-sys, risc0-build-kernel, risc0-groth16-sys) and sppark | | the workspace builds the engine's driver (prover/host) |
| prover/methods (Cargo.toml, build.rs, src/lib.rs) | risc0-build of prover/methods/guest: METHOD_ELF, METHOD_ID, and `profile.rs` (LIBZYNC, NETWORK, COINLOG, TRIE, UTXOHASH, TXLOG, TEST_LINEAGE, LIBZYNC_ASSUMEVALID) | | YES: the profile constants (`methods::NETWORK` 20, `COINLOG` 18, `TRIE` 15, `TEST_LINEAGE` 4, ...) and `METHOD_ID` (76 refs: the default self_id, prep's lineage checks, `method-id`) |
| prover/host (14,325 lines) | risc0-zkvm 3.1.0 `prove` (+ `cuda`), risc0-binfmt, risc0-zkp: execution, proving, receipts, segments, the RISC Zero handover | Receipt 108, METHOD_ID 76, ExecutorEnv 43, ReceiptClaim 30, METHOD_ELF 27; by file: lz.rs 153, seg.rs 94, main.rs 65, lz_coord.rs 27, verify.rs 24, bench.rs 17, sink.rs 11, chunks.rs 10, prep.rs 8, trie.rs 7, engine_backend.rs 7 | the engine path is chosen per run by HAZYNC_ENGINE and interleaved with the RISC Zero path (lz.rs `lz_prove_range_cmd` l.1197-1780: `eng` branches; `lz_join_cmd`; `engine_export` executes the chunks on RISC Zero for their journals and cycles) |
| prover/host commands that are RISC Zero only | seg-* (seg.rs, 2,968 lines: the segment-distributed prover), segment-size, segment-mem, receipt-digest, exec-time, chunk-profile, agg-chunks, agg-exec-fake, prove-chunk, exec-chunk, journal-sha, signet-check-test, libzync-snark-wrap (Groth16), xver-selftest, xver-encode, libzync-handover (the RISC Zero cross-version handover), libzync-hints (hints by RISC Zero collect runs), core-vectors / core-vectors-export (bench.rs) | | no; but core-vectors and the regtest rig are CONSENSUS EVIDENCE (Core's 22,666 vectors, the 96-case rig): they must be ported to the engine before their RISC Zero runners go (engine/results/core_vectors used `engine-export` streams) |
| prover/zyncapps/methods (Cargo.toml, build.rs, src/lib.rs, guest/, hashbench/, triebench/) | risc0-build of the zAPP guests: ZYNCAPP_*_ELF / _ID | | no: engine/zapps/assemble_guest.sh reads each zAPP's sources and its Cargo.toml's [dependencies] only (keep those) |
| prover/zyncapps/host (3,277 lines; main.rs 75 refs) | ExecutorEnv / default_prover / receipts beside each `--stream` path | | the `--stream` paths (hints-prove, utreexo-prove, utreexo-tip-prove, zeroconf) are the engine's input writers |
| prover/methods/guest, prover/zyncapps/methods/<zAPP> | guest SOURCES | | YES: assembled for the engine (engine/guest/assemble.sh copies the whole guest dir, then replaces Cargo.toml / build.rs and deletes Cargo.lock) |
| vendor/risc0-*, vendor/sppark | the six patched crates | | only through the prover workspace's risc0 graph |
| history/verifier, verifier-ffi, verifier-wasm, rangestate | the CORE design's (v1) verifiers, a record (history/README.md: not built, tested or run by CI); risc0-zkvm from crates.io | | no; only comments point at them |
| experimental/libzync-bench/* (host, methods, xver, statement, liveness-anon, deposit tools, ...) | RISC Zero bench crates and their scripts, the measurements' provenance | | no; records (own workspaces, crates.io risc0) |
| engine/journal/compat | the C1 check against risc0-zkvm 3.1.0 | | a one-off check, kept as the record of the byte-identity claim |
| accumulator, zapps/assume, zapps/utreexo (Cargo.toml mentions) | comments / dev notes | | check each: no risc0 dependency in their graphs since C4 (cargo tree) |
| reproduce/: libzync.Dockerfile, libzync-ids.sh, LIBZYNC_IDS, METHOD_ID, GUEST_ELF_SHA256, LINEAGE.tsv, pin-libzync-ids.py, Dockerfile | the RISC Zero images' reproducible build and pins | | no (engine: engine.Dockerfile, engine-ids.sh, engine-records) |
| CI: windows-prove.yml | builds host.exe WITH the RISC Zero guest embedded, checks it against LIBZYNC_IDS, proves block 1 | | the Windows CPU product host (release asset hazync-host-x86_64-windows-cpu.zip) comes from it today |
| CI: windows-installer.yml l.281-283, release-sign.yml l.103-120 | read LIBZYNC_IDS / METHOD_ID | | partly (the engine paths were added 28.11 beside them) |
| CI: adversarial.yml l.153 | reads LIBZYNC_IDS | | check |
| CI: cuda-arch-audit.yml, cuda-probe.yml | RISC Zero CUDA diagnostics | | no |
| prover/host/tests/data/risc0_vectors | RISC Zero receipts for the host's receipt tests | | no |
| THIRD_PARTY_NOTICES.md | RISC Zero entries | | while anything above builds on it |

## What goes, what stays

Goes: every RISC Zero execution / proving / receipt path in prover/host and prover/zyncapps/host; prover/methods'
risc0-build (the profile constants move to a build script with no zkVM in it, so they cannot drift from the engine
guest's: they are the same env vars and the same `libzync/src/profile.rs`); prover/zyncapps/methods; vendor/risc0-* and
vendor/sppark; the RISC Zero-only commands; history/verifier, verifier-ffi, verifier-wasm (and rangestate if only they use
it); reproduce's RISC Zero pins and Dockerfiles; the RISC Zero CI (windows-prove.yml's guest embed, cuda-arch-audit,
cuda-probe); the risc0_vectors tests; THIRD_PARTY_NOTICES' RISC Zero entries.

Stays: the guest sources (the engine assembles them); each zAPP guest's Cargo.toml (assemble_guest.sh reads its
[dependencies]); prover/host as the engine's driver (prep, engine-export, lz_prove_range on the engine, the board's
commands prove-range-bridge / fold-range / verify-any / method-id / seg-po2, reorg, rewind, reforest, bridge, the coin log
store, history); the zAPP hosts' `--stream` writers; experimental/ (records); engine/journal/compat (the record of C1).

## Order (each a commit with every suite passing)

1. **Serde without RISC Zero.** prover/host and prover/zyncapps/host encode and decode the guests' words with
   hazync-journal (byte-identical to risc0-zkvm 3.1.0's, engine/journal/compat), not `risc0_zkvm::serde`. Checked: the
   same bytes from both on every type the hosts write, while RISC Zero is still there to compare against.
2. **The zAPP host engine only.** prover/zyncapps/host keeps its `--stream` writers and its non-proving tools; the
   ExecutorEnv / prove / receipt paths, `id`, hashbench, triebench and headers-exec / headers-prove (the engine's header
   input is engine/zapps/headers' hdrtool.py) go; prover/zyncapps/methods goes. Checked: every `--stream` file
   byte-identical to the RISC Zero-era host's on the repo's fixtures.
3. **history/verifier\*.** Deleted with their comments' pointers updated (history/README.md).
4. **prover/host engine only** (the large one, several commits):
   a. the profile constants from a zkVM-free build script (`methods` becomes a profile crate: no guest build);
   b. `METHOD_ID` replaced: the host's image id is the engine's, from its key dir or verifying key
      (engine_backend::image_id_words / engine_image_words, as `method-id` already prints); prep's default self_id and
      `leaf_has_undo(&METHOD_ID, ...)` take that id (prep states are untrusted starting points; the engine path already
      overwrites self_id: lz.rs l.162-170);
   c. `engine-export` without RISC Zero: the chunk journals and the aggregate's inputs from `hzengine execute` (the
      engine's own chunk journals are identical to RISC Zero's: results/node_format2), cycles replaced by the engine's
      cost (the scheduler's ranking);
   d. the RISC Zero-only commands deleted (list above), after the core-vectors and regtest-rig runners are on the
      engine (they are consensus evidence and must not be lost);
   e. lz_prove_range / lz_join / hints: the RISC Zero branches deleted, the engine path made unconditional
      (HAZYNC_ENGINE required);
   f. risc0-zkvm, risc0-binfmt, risc0-zkp and the vendored crates out of the workspace.
   Checked at each step: the engine images' ids unchanged (the guest sources are not touched); `engine-export` streams
   byte-identical to before on the repo's block fixtures; the host's unit tests; the board dry run's commands on the
   box (prove-range-bridge, fold-range, verify-any, method-id) on blocks 1..8 as results/board_dryrun.
5. **CI and reproduce.** windows-prove.yml builds the CPU product host with no guest (the Windows driver), the RISC Zero
   pins and Dockerfiles go, release-sign / windows-installer / adversarial drop their LIBZYNC_IDS reads.
6. **Notices and docs.** THIRD_PARTY_NOTICES, CUTOVER.md C8 and its log.

## Decisions this needs (before step 4)

- Step 4d deletes the RISC Zero runners of two pieces of consensus evidence (Core's vectors and the regtest rig). Their
  engine ports come first: results/core_vectors already ran Core's 22,666 vectors on the engine image; the regtest
  rig's engine run is queued (ROADMAP: "the regtest harness re-proved on the engine").
- The Windows CPU product host (release asset) is built today by windows-prove.yml with the RISC Zero guest inside;
  after step 4 it is a plain Rust build, and its engine path on Windows is untested until the Xesktop session (2.9).

## Status

**Step 1 done (28.11.2026, bd10e3cd):** both hosts encode with hazync-journal. prover/host's test `sink::serde_identity`
writes the anchor state, the witness and three chunk splits of five real blocks (130,000, 140,000, 741,000, 962,000,
965,500) through hazync-journal and through risc0_zkvm::serde: byte-identical. Host suite 109 passed, 1 ignored.

**Step 2 done (28.11.2026):** prover/zyncapps/host writes engine streams only, and prover/zyncapps/methods (risc0-build,
the coin guest, hashbench, triebench) is gone; the zAPP guest directories (headers, hints, utreexo, utreexo-tip,
zeroconf) stay. Gone from the zAPP host: `id`, headers-exec, headers-prove, bench-hash, bench-trie, verify-zeroconf,
prove-coin, inspect-coin, verify-coin, hints-verify (a stub), and every RISC Zero prove / execute / receipt path; each
`*-prove` needs `--stream`; a core statement is an engine `.journal` or a state JSON. risc0-zkvm and bincode are out of
its graph. Checked: the RISC Zero-era host (the same source with RISC0_SKIP_BUILD=1, before the change) and the new one
write byte-identical streams for hints-prove at 200 and 150, HINTS_TAMPER=lineage and core_id, utreexo-prove at 200
and 100, UTREEXO_TAMPER=lineage, utreexo-tip-prove 101..200 (native and forest sources), prove-zeroconf (the regtest
fixture, record 0, 12,700 bytes), and the same core-info line (mainnet 1..200 blocks, the engine chain's state_200.json,
its utreexo base journal at 100). The engine journal of block 200 as the core input gives the same hints stream as the
state JSON. The binary is byte-identical before and after removing the methods crate from the workspace.

Left by step 2, RISC Zero harnesses that drive the removed execute / prove paths (run by hand, or by the RISC Zero CI
jobs below), to port to `--stream` + `hzengine execute` or delete with step 4d / 5:
scripts/libzync/hints_test.sh, scripts/libzync/hints_verify_test.sh, zapps/utreexo/tip-exec-tests.sh,
zapps/utreexo/tip-source-tests.sh, coordinator/deploy/utreexo (the follower, its check and tests) and
coordinator/deploy/headers/test/test_follow.sh (the RISC Zero followers: 3.4 rebuilds them on the engine),
reproduce/libzync-ids.sh (`zyncapp id`), and adversarial.yml's RISC Zero jobs (reproducible-image-id, libzync-hosts,
libzync-chain, libzync-blocks, libzync-vectors, libzync-rig, libzync-signet; they run on main and pull requests, not on
pushes to engine/openvm).
