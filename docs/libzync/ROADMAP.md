# libzync: roadmap to the freeze and beyond (started 11-03, status updated 2026-11-12)

libzync is the engine of Hazync v2 (Hazync v1, the original Core design, is in [`docs/history/`](../history/)).
**Where it stands on 2026-11-12:** v0.24.0 is released on RISC Zero 3.1.0 (images `8888d59c` mainnet, `d4210dae`
signet, `2db34e12` header chain; `reproduce/LIBZYNC_IDS`); v0.23.0 (`863c9ce3`, 3.0.5) is withdrawn. The public
board switched to v2 on 2026-11-12 and restarted from genesis (N2). The header chain is proved in one receipt and
followed at the tip on CPU (K1, N3). zyncAPP pages are live on hazync.org (R4). The Windows CUDA host proves natively
(R3). What is still open is in **N** and **O** at the end. Entries below keep the dates and image ids they were
written with: an id other than those three is a historical (3.0.5 or laptop-built) image.

Everything built, discussed or promised so far, in build order. Status words: **done**, **building**, **next**,
**experiment** (built behind a flag, measured, then decided).

⚠ **Testing policy until phase H (historical: phase H is done):** unit tests and compiles only. No long suites (rig, handover, signet from genesis)
until everything is built and reviewed; then one full test pass on the final image.

## A. Chain identity

| # | item | status |
|---|---|---|
| A1 | guest pins one checkpoint (`checkpoint.rs`: mainnet 970,000, signet 325,000); verifiers give DEMO below its height | done (c8 compiles, 9d9adb54 before B1) |
| A2 | `--expect` / `--mempool` / `--mempool-url` in both verifiers, refuse mistyped flags, unit tests | done (`3785f56`, `8e47047`) |
| A3 | host verifier `--bitcoin-cli` | done |

Minimum work stays a verifier check: every proof below the tip is low-work by construction, so the guest cannot
refuse it.

## B. zyncAPP coin facts (ZYNCAPPS.md, COINLOG.md §6)

| # | item | status |
|---|---|---|
| B1 | `libzync::facts`: coin leaf, unspent / spent-by, block in chain + state at a height, tx in block, tx parsing, `check_core_statement` | done |
| B2 | host writes `history.bin`; `zync-path coin` / `find` produce coin facts | done |
| B3 | coin journey: creating tx in a proven block, the coin leaf from its output, the spending tx (`zync-path journey`, `--journey`) | done |
| B4 | `swiftsync-verify --coin / --journey`, wasm `verify_facts`, C ABI `swiftsync_verify_facts` | done |
| B5 | zyncAPP `coin` guest (prover/zyncapps): unspent/spent at the core tip, core receipt verified in the zkVM; image 26e09637 | done. ⚠ (round 11) it commits the LEAF it checked, so "unspent" is about that 32-byte leaf: a spent coin's flipped leaf also passes; a consumer must compare `coin` with its own `coin_leaf`. ✅ **11-07: version 2** (image 3f503000): the guest takes the coin's FIELDS (txid, vout, value, script, height, flags, creation MTP), computes the leaf and commits the fields (kind 0x5A430002; verify-coin refuses version 1); `prove-coin --preimage`, `--execute`; tested on mainnet 1..200 (spent coin, unspent coin, both mis-claims refused in the guest, wrong / missing preimage refused: `zcoin/RESULT_coin_v2.txt`). ⚠ 26e09637 / 3f503000 are laptop builds on risc0 3.0.5: no `coin` zyncAPP id is published or pinned for 3.1.0 (`reproduce/LIBZYNC_IDS` holds the core and header-chain ids only). The website's Coin check uses trie facts against a core proof, not this guest |

## C. Production host

| # | item | status |
|---|---|---|
| C1 | disk-backed coin-log store (`HAZYNC_COINLOG_DIR`, DiskNodes) + C1b on-disk position index (DiskIndex) | done (history.bin replay is still in memory) |
| C2 | `zync-serve` path service (GET /tip, /coin, /find); payment hook not yet | done |

## D. Transaction log (txproofs/README.md)

| # | item | status |
|---|---|---|
| D1 | Phase A: speculative chunks (zero hash/time) accepted from CSV on when flags need no hash; host `HAZYNC_SPECULATIVE` | done (image c9tx df86c392) |
| D2 | aggregate record selection (`HAZYNC_TXLOG_ORDER`, each record once) | done |
| D3 | Phase B (context-free records): NOT needed — Phase A waste measured 7.5% of pre-proved inputs over 8 blocks (txproofs §8) | dropped |
| D4 | `fetch_template_rpc.py` (template -> speculative fixture); waste measured 7.5% (txproofs §8) | done |
| D5 | zyncAPP `zeroconf` guest: speculative chunk record + every input unspent at the tip | building |
| D6 | Ghost proof-ready blocks: the pool runs D4 on its own template (Ghost-side integration; Ghost infra is separate) | after D4 measures |

## E. Proof of absence

| # | item | status |
|---|---|---|
| E1 | keyed trees for absence (outpoint / script), built as guest features otree / stree and MEASURED on 10 blocks: +55.6% one tree, +110.7% both, almost all in the aggregate (absence/README.md); key-prefix sharding measured: no saving in work, only splits serial execution — not pursued; history-keeping script tree measured +49.7% alone / +100.1% with the outpoint tree, BUT undercounted (within-block spent coins not inserted; README correction 11-04) | measured |
| E1b | corrected history tree (every created coin, within-block spends inserted marked) and the counting trie (one two-level trie: address then coin, history kept, per-address unspent count and value; `ttree.rs`), images c10h2 / c10t. MEASURED (9 modern blocks): corrected history tree +60.5%; **counting trie +51.3%, every fact in one tree** (counts ~free; 1,024 coins per address: +44% / +82% on 962,000 / 750,000) | measured |
| E1c | counting trie TRIMMED (one pass over the witness, hashes in place, cheaper chunk keys, one-compression address leaf): **+28.0% over 9 modern blocks (was +51.3%), 962,000 8.40x vs main** (c10t5: full 256-bit address binding, coin key fix); at its hashing floor (absence/README.md) | **CHOSEN 11-04 (user): the path forward** |
| E2 | integrate the counting trie (c10t5 design) into the core: trie root in ChainState bound proof to proof (not from the witness); the trie updated in its OWN receipt in parallel with the chunks and verified inside the core proof (split measured: serial aggregate 192 M -> 99 M, whole-block cycles +2.4%; pinned in the core image like the chunk program); host trie store; absence / history / balance verifiers (cli, wasm, C ABI); zync-serve endpoints | **done 11-05.** Steps 1-2 (7ab30d4, c10e5 ce96f2ec): trie as guest mode 17, receipt verified in the aggregate, root in the chain state; negatives refused; product +19.9% over the pre-hunt no-tree base on 9 modern blocks, 962,000 9.09x vs main. Step 3 (7e00760, 1c63de5; c10e7 026ebf10): host trie store (`libzync::trie_store`, snapshots `state_<h>.trie` beside the coin log, all-or-nothing apply, undo), proved and committed per block in prove-range; test genesis..200 + resume at 120 (same root) + tampered snapshots refused + 4 negatives refused (`trie_store/RESULT_c10e7.txt`). Step 4 (82a31d6, 806d9e1): coin / balance / history facts (`libzync::trie_fact`) in swiftsync-verify `--trie`, the C ABI and WASM (`"tries"`), `zync-path trie`, `zync-serve /trie/*`; checked on the real 0..200 run (block 9's coinbase spent at 170 by tx 1 input 0; 5 coins ever paid to that script, 28 BTC left). Cleanup: the wire's hint-bit field, the hints-file branch of every verifier and the v1 bundle removed; ChainState fields renamed `trie_root` / `live_set` / `live_hash` (JSON aliases read the old names) (bad3c7d; c10e8 e8f9ac1b: the store test again, identical, `trie_store/RESULT_c10e8.txt`). Review round 1 (a4a7c40): trie facts carry their claimed answer (`expect`), the C ABI requires one; product GPU runs checkpoint the trie. ✅ 11-05 mainnet scale: the store is an ARENA of 64-byte records, in memory or memory-mapped on disk (`TrieStore::open_dir`, host `HAZYNC_TRIE_DIR`), copy-on-write (an apply is all or nothing; slots freed only after a flush, so a crash leaves the last flushed trie); disk = memory block for block (roots, witnesses, snapshots), crash test passes. c10e12 (a9dd2da5): the store test again, identical, and native prep 1..200 with the trie on disk byte-identical to the proved run (`trie_store/RESULT_c10e12.txt`). Sizing: 4.04 nodes / 258 bytes per coin on a synthetic chain with little address reuse; real mainnet bytes per coin to be measured by a prep run (estimate hundreds of GB: needs NVMe on the prep / tip box) |
| E2b | instant IBD: the chain state's ECMH total tracks the LIVE coin set (`utxohash`), so a new node verifies a downloaded UTXO set (~10 GB) against the proof. Measured +4.56% padded over 9 modern blocks (+1.2-1.8% on recent blocks). **CHOSEN 11-04 (user)**; part of E2 | chosen |
| E2c | consolidation (after E2a batches 2-3, before E2 integration and the reviews): ONE product configuration (coin log + counting trie as a parallel receipt + live-coin hash; signet / regtest / txlog kept); delete the dead experiments from the guest (otree / stree / htree / itree, design B + hints file, msm / field-bench / liftx-accel / tx-cycles, trie inside-vs-split switch); split `libzync` into the core library (coin log, trie update, facts, wire) and tooling / bench code kept out of anything that ships; break host main.rs into modules (prover, verifier, tools; benches out of the main binary). Cost table + unit tests green after each step | **done 11-05** (10161bf, 361e4f5, b26ca4f, host split): product image 16d9409b; every step's aggregate journal identical to batch 2's apart from the image-id fields. (The hint-bit field, hint hash and hints-file branch went in E2 step 4.) ⚠ pre-existing host test failure smt_bridge::a_fully_spent_coinbase_can_be_duplicated_with_no_special_case (BIP30 coinbase duplication in the bridge SMT) — review rounds |
| E2a | cycle hunt across all core units (chunk, aggregate, trie guest): profiles in `~/libzync-gpu/e1/prof`, three read-only agents, each candidate measured on the cost table before it lands. Batch 1 (whole blocks -0.8 to -12.5%), batch 2 (aggregate -28.5%, whole -6.4%), batch 3 (trie guest -11.3%) | done 11-04 (more trie work possible later) |

## F. main branch

| # | item | status |
|---|---|---|
| F1 | port the `is_base` forgery fix and the BIP68 fix to `main` | **closed as superseded**: #668 closed when libzync replaced the CORE guest on `main` (R1, R1a) |

**Release: libzync onto `main` and the public infra (user, 11-07). Status 11-12: R1-R4 done, R5 open.** Order to agree with
the user; every release's contents are gone through with the user before it is cut.

| # | item | what exists | what is left |
|---|---|---|---|
| R1 | merge `libzync` into `main` | `libzync` is 307 commits ahead, main 1 (#669). CI has never run on `libzync` (main + PRs only) | a PR from `libzync` (runs the whole CI for the first time: expect reds to fix), main's #669 merged in, F1 (#668) decided; the release (`release.sh`) builds and publishes the libzync host + the frozen image ids (H7, waits for the container builds). ✅ **done**: merged as #670 (11-08); released as v0.23.0 (withdrawn) and v0.24.0 (11-12, risc0 3.1.0) |
| R1a | ⚖ **DECIDED 11-07 (user): libzync REPLACES CORE in the build (option a)** | the CORE guest's own sources differ on `libzync` (main.rs +739/-, verify_input.cpp +620, msm.rs removed, 28 files), so the merged tree can NOT rebuild the canonical CORE id 37987b85 (since superseded) and CI's `reproducible-image-id` (pinned to it) goes red | (a) RECOMMENDED: libzync replaces CORE in the build; CI pins the libzync ids (H7); CORE stays usable as its published release binary (37987b85) + a tag, which is all the paper's same-card CORE re-measure needs; (b) keep CORE buildable by restoring its guest files byte-identically as a separate crate (upkeep for an image nobody proves with). ✅ **done, (a)**: CI's image checks read `reproduce/LIBZYNC_IDS` (`adversarial.yml`, `release-sign.yml`); CORE stays as release v0.22.1 (`37987b85`) |
| R2 | keep donations and **donate to prove** (sponsor a block range) | `server.py`: BTCPay invoices, sponsor holds and queue, the sponsor bot, `/api/donations`; all keyed by block ranges | check on a libzync coordinator (HAZYNC_LIBZYNC=1): a sponsored range is a run of leaves + joins, so the semantics carry over; run the sponsor / donation tests in libzync mode; the board text says libzync. ✅ **done at the switch (11-12)**: the v2 board keeps claims, sponsorships and the sponsor bot (`HAZYNC_LIBZYNC=1`, ON again since 11-12; caps $50/run, $10/hr, 4 pods, pods under $1/hr) and the BTCPay drop-in (`coordinator/deploy/libzync/SWITCH.md` step 3); the v1 sponsorships and donations record stays in the read-only v1 database |
| R3 | the Windows app, so anyone can help | `tools/win-gui` (GUI around the `hazync` worker; runs end to end on real Windows) | a Windows libzync host. ⛔ GPU proving on Windows is blocked by #631 (CUDA cooperative launch under WDDM, still open); CPU-only is slow (a modern leaf is ~9 GPU-minutes). Decide: fix #631 first, or ship Windows as CPU / small-block work + Linux for GPUs. ✅ **done**: the libzync CUDA host proves natively on Windows (RTX PRO 6000 under WDDM: block 170 in 319 s, 400,000 in 823 s, both verified on Linux; `experimental/libzync-bench/windows/RESULT_xesktop_rtxpro6000.txt`, measured on 3.0.5 image `863c9ce3`); shipped as the release asset `hazync-host-x86_64-windows-cuda.zip`, and the GUI's supervisor pins `8888d59c` (`tools/win-gui/supervisor.py`) |
| R4 | zyncAPP front ends on the website | the verifier in WASM (`swiftsync-verify-wasm`: coin, balance, history, absence facts against a core proof), `zync-serve` endpoints, two zyncAPPs (`prover/zyncapps`: coin, header chain) | a web page that fetches a fact + the core receipt and checks it in the browser (no trust in the server), one per zyncAPP; hosted with the board. ✅ **done (11-12)**: hazync.org `/zapps/` (the list), `/headers/` (header chain), `/zapp/` (Coin check). Open: Coin check reads the fixed mainnet 1..200 proof, not the board's latest (O2) |
| R5 | zyncAPPs as their own applications (Windows / Linux) | the C ABI (`swiftsync-verify-ffi`) and the CLI | a small desktop app per zyncAPP on the C ABI (or the WASM in a webview); packaging for both OSes |

## G. Docs

| # | item | status |
|---|---|---|
| G1 | spec text for each item as it lands (SWIFTSYNC_PROOFS, COINLOG, ZYNCAPPS, txproofs) | with each item |
| G2 | **final repo cleanup (user, 11-05): a concise, readable repo, minimal docs and tools, no fluff, precise info.** Every experiment-era doc, script, bench and result moves to `experimental/` (or is deleted): the retired design-B docs (SWIFTSYNC_PROOFS, CORE_HOOK, UTREEXO_FORMAT), this ROADMAP, the reference scripts in `scripts/libzync` (keep only what a test or a reproducible id needs), the zyncapps bench guests (hashbench, triebench), checked-in test outputs. Live spec content those docs still carry (the coin leaf, UTREEXO_FORMAT §2; the genesis anchor rule) is folded into a few short specs first, and code comments re-pointed. Then decide the historical names (`swiftsync-verify*`). Hazync's own docs (docs/*.md, ~25: coordinator, fleet, proving, soundness, ...): libzync IS Hazync (the new library), so each doc that is relevant and current stays or is updated, stale ones go, and the many overlapping ones are condensed / combined into fewer | ✅ **DONE 11-06** (d7929e0, da06835, b32d7a4, efcdc3f, 4a97842): docs/ 27 → 8 (OVERVIEW, OPERATIONS, COORDINATOR, SECURITY, BUILD_AND_RELEASE + SPEC, the generated COORDINATOR_REFERENCE, the index), originals in docs/history/; LIBZYNC.md rewritten; SWIFTSYNC_PROOFS / UTREEXO_FORMAT trimmed to live spec; CONSENSUS_RULES / ZYNCAPPS de-staled; check-versions no longer passes on a missing file or line; libzync crates now in CI. **BEFORE the freeze (user 11-06).** Scope decided 11-06: original-Hazync CODE stays (coordinator, tools, CORE path); its docs are condensed into a few current ones, dated ones to docs/history; experiment-era docs, outputs and one-off scripts MOVE to `experimental/` (nothing deleted); fuzz crates and the `swiftsync-verify*` names left as they are (names decided with the paper). Guest-compiled files untouched (image) |
| G3 | **the paper, written WITH the user** (not drafted alone): what libzync proves, how, measured costs, soundness argument, limits. Kept PRIVATE, out of this repo (user 11-07) | after G2 |

## H. Path to the core freeze (the user's gates, in order)

1. ~~E1 tree decision~~ — **done 11-04: the counting trie (E1c) is the path forward.**
2. **E2a** cycle-hunt batches 2-3, then **E2c** consolidation (one product configuration, dead experiments deleted, library split), then **E2** integration (trie as a parallel receipt inside the core proof, live-coin hash) — moves the image.
3. Several cold review rounds on everything since `00b7ac2` (feedback: reviews before any freeze). Round 6 (11-06,
   the BIP30 re-creation fix d6f8b4b): no accept / root divergence reachable; model / store depth aligned with the
   guest, undo refuses after a re-creation, tests added (838bf5b).
4. One full test pass on the final image: rig vs Core, Core's own test vectors (I1), handover, mainnet 1..200, signet
   from genesis, edge blocks, cost table. **c13 (f769cb8d) GREEN on every step 11-06** (`handover/RESULT_c13.txt`:
   rig 96/96, handover 16/16, mainnet 1..200 and signet 1..2600 = both references, cost identical to c10e12, edge
   10/10). c14 (9d28cd9: a collect run exits with COLLECT_EXIT instead of a panic; the host checks every exit code)
   gets the short pass (`SN=100 retest_final.sh`): the change touches no network rule or long-chain state.
   **c14 (bbf82800) GREEN on every step 11-06** (`handover/RESULT_c14.txt`): unit tests, rig 96/96, handover 16/16,
   mainnet 1..200 and signet 1..100 = both references with 0 panics, edge 10/10, cost vs c13 +2 aggregate cycles and
   about -2,000 chunk cycles a block (<0.01%). **c14 is the freeze candidate** (reg 3036d379, regN fcde3e5a, sig 219215fb;
   all laptop builds on risc0 3.0.5, historical).
5. **GPU proving benchmarks** on the final image (user's trigger). Ready: run B (`gpu/gpu-libzync-multi.sh`, product image) dry-run on CPU
   with c10e12: hints 45 s (2 threads), execute check passes, EXPECT_CYCLES=64896838 (block 962,000 aggregate).
   **On c14: EXPECT_CYCLES=64896840** (the cost table, RESULT_c14.txt). ✅ **DONE 11-06** (`gpu/GPU_RESULTS.md`, $3.72):
   block 962,000 on one PRO 6000 in **537 s** (406 s chunks + 132 s aggregate incl. the 51 s trie proof; design B 517 s),
   every receipt verified; a first pod on a loaded host took 894 s (the host, not the image: same bundle).
6. **Before the freeze (user, 11-06): a review of everything since round 6, then the G2 repo cleanup, then a full cold
   review of the cleaned repo; a retest only if a fix moves the image.** Cleanup keeps guest-compiled files untouched
   (their bytes, comments included, are the image). Proof age is NOT an expiry (a proof is valid forever): every coin / trie / journey answer already names its height
   (checked 11-06; a trie fact must be about the proof's tip), and the app decides how old is too old. Nothing to build.
   **Full cold review of the cleaned repo (11-06), four reviewers:** guest CLEAN (no host input gets an invalid block,
   wrong state or forged chain proof accepted); docs vs code: 10 mismatches, fixed (2403c4f); verifiers: H1 the trie
   coin key (txid with vout XORed into 4 bytes) aliases (txid ^ d, vout ^ d), so a trie coin fact could certify an
   outpoint that never existed: FIXED verifier-side, a coin the trie holds needs its creating transaction (`origin`,
   a tx fact; `zync-path tx`), image unchanged (user asked for a recommendation: verifier rule, no guest change); M1
   FFI/WASM took the trusted tip from the facts JSON: now a separate argument, refused inside it; L1 host
   `--anchor` hashed without zeroing anchor_id: fixed; host: a resume with a disk trie always panicked (fixed),
   receipts / states / history.bin now written atomically, dev-mode runs say so on their DONE line, an existing disk
   coin log is never wiped without HAZYNC_COINLOG_FRESH=1. **The three open items, FIXED 11-06 (user):** a
   composite receipt (it nests without bound: a hostile one overflowed the verifier's stack) is refused from its first
   4 bytes before decoding, in swiftsync-verify (CLI / FFI / WASM) and host libzync-verify; the disk coin log keeps a
   multi-block journal until the block's ONE commit point (both stores saved), so a crash between the two saves, or
   anywhere in a libzync-prep range, resumes from the last saved height (prep now also saves every 1,000 blocks in disk
   mode; a record carries its height, which also ends the false rollback of a block with no appends); AGG_EARLY moves
   stale chunk files aside at start, checks each file's claim while waiting and times out
   (`HAZYNC_AGG_EARLY_WAIT_S`). Tested (cb5d3e5 + `HAZYNC_PREP_CONTINUE`): host rebuilt, image still bbf82800; resume 1..5 -> 10 in memory and with a disk trie, prove-range killed between the two saves then resumed, prep killed at 25 and restarted from 20 to 60: every state byte-identical to the proved chain's; libzync 31, swiftsync-verify 16, ffi 3, host 68, wasm vectors 6/6. Still by design: FFI/WASM take `now` from the caller; a regtest host prints OK. **For the next image (guest files, so not in c14; round 9):** `field_bigint2.h`'s comment says elements are canonical, the code is lazy ([0, 2^256), correct); and make the parse cache's reliance on the bump allocator (no address reuse; `verify_input.cpp` cached_tx) a build-time check instead of a comment.
   **Review rounds 9-11 (11-06/07, user asked for more):** no guest change. Clean: consensus completeness vs Core v28
   (rule families, heights, flags, per-chunk vs per-block); the guest's C/C++ (batch signatures, lift-x, parse cache,
   Core stand-ins, exceptions); every patch the build applies (Core 0001 0002 0009 0010 0101; secp 0012 0013 0005 0006
   0008 0102 0103; 0014 not applied); the facts library (no false fact once the coin is bound). Fixed (host / verifier /
   docs): the disk coin-log journal through three rounds (checksummed LZPEND03 records, the one commit point, recovery
   from any crash or power loss by undoing every journaled block back to the saved state), directory locks, the disk
   trie resume, the C ABI `expect`, the standalone CoinFact needing the caller's preimage for UNSPENT, CONSENSUS_RULES
   (the checkpoint as a STRICTER row). Crash test: 7 scenarios, all byte-identical to the proved chain. 45e2d2d. **Round 12** (on round 11's fixes): the
   undo-to-state recovery confirmed sound node by node; fixed a regression (unspent journeys refused by the preimage
   rule), the LOCK surviving a fresh store's wipe, overlapping run directories refused, prep from a mid-chain state
   naming its mode (CONTINUE or BASE) before any store is opened. Crash test: 10 scenarios, each refusal checked for
   its reason; host 69, verify 16, ffi 3; image bbf82800.
7. **Freeze: ✅ DECIDED by the user 11-07: c14 is frozen.** Guest source as of 9d28cd9 (unchanged through b732263);
   laptop-built ids c14m bbf82800 / sig 219215fb (regtest 3036d379 / fcde3e5a are test-only). The PUBLISHED ids are the
   reproducible container's (reproduce/libzync.Dockerfile; an id built elsewhere differs by design): built twice,
   checked against the laptop image's behaviour, then pinned in reproduce/LIBZYNC_IDS and the verifiers (H7). ✅ done
   (H7 below); since 11-11 the pinned ids are the risc0 3.1.0 rebuild of the same source (`8888d59c` / `d4210dae`).
8. After freeze: container ids built twice, LIBZYNC_IDS + verifier IMAGES tables (cli/ffi/wasm), vectors regenerated
   from GPU run A, proposal page refresh. A full mainnet proof is the user's trigger. ✅ **status 11-12:** the
   IMAGES tables hold `8888d59c` / `d4210dae` (`swiftsync-verify/src/lib.rs`, `swiftsync-verify-ffi/src/lib.rs`), the
   header-chain id `2db34e12` (`swiftsync-verify/src/headers.rs`); vectors regenerated on 3.1.0
   (`swiftsync-verify/vectors/mainnet_1_200_8888d59c*.rcpt`). Proof age: decided in step 6 (no expiry). Proposal
   page refresh: not checked. A full mainnet proof: still the user's trigger; the board proves from genesis. ⚠ review round 3 (11-05): until
   then the built-in IMAGES (9f469930 / 067822b9, from c1101a3 10-31: ChainState v1, profile 1) verify NOTHING current,
   so OK is reachable only via `--image` / `image_hex`, and the table's `checkpoint_height: 970_000` describes an image
   with no checkpoint: replace both with the frozen product images (cli, ffi `IDS`, wasm `image_id`). Also decide a
   proof-age policy (today a genuine year-old proof is OK; facts are "as of its height").
9. **G3 the paper with the user** (G2 cleanup moved before the freeze).

### H-B. Backfill design: anchored blocks + one join tree (agreed with the user 11-05)

The original Hazync's shape (independent block proofs folded into one chain proof; its prove_range / fold_range) was
right; the separate anchoring stage goes. Today libzync is strictly LINEAR (block N's aggregate verifies N-1's
receipt): fine at the tip, a serial bottleneck for a whole-chain backfill.

1. Prep (host, native): the exact chain state before every block, S0 (genesis) .. S_tip: coin log, trie, BIP30 tree,
   block forest, work, times. Not trusted, only starting points.
2. Leaves: any block, any order, any number of GPUs: block h proved as a base step on S(h-1) ("from the state whose
   digest is anchor_id, block h is valid and gives S(h)"). Already supported (the anchor path).
3. Joins: a NEW guest mode (~50 lines): env::verify left and right (same image), require sha256d(left journal) ==
   right.anchor_id, same kind / profile / lineage; commit the right state with the left's anchor_id. The full-journal
   digest covers every state field at once. A binary tree over N blocks is N-1 joins (= the N-1 links), ~20 levels for
   ~975 k blocks, every level parallel. A wrong host state only fails a join (digest), never forges.
4. Tip: unchanged (linear step, or prove on S_tip and join onto the root: the same statement).
5. Published: ONE receipt (root, genesis -> tip; optionally Groth16-wrapped) + its chain state. WASM verifies that one
   receipt (pinned image, start == the genesis anchor else DEMO, tip vs a trusted source); "block X is valid" = a
   block-forest path from X to the root's block_roots; coin / trie facts against the same state.
6. Across an image change: the handover step, not a join.
7. (review round 5, 11-05) The LINK digest excludes `anchor_id`: a base step commits anchor_id = sha256d(the anchor's
   journal with anchor_id zeroed) and a join requires that of the left part. With anchor_id inside the digest a leaf
   on the host's S(h-1) linked only to a left part that started where S(h-1) says, so only a left comb from the
   range's start could be joined, never a balanced tree. The genesis state's anchor_id is zero: its pinned id is
   unchanged. anchor_id still records where a chain started (genesis, for the verifier).
8. Leaves in any order need the trie WITNESS for block h against the trie at h-1. Prep with the trie in memory can
   snapshot every height; on disk (one store advanced in place) prep must write each block's witness (small) as it
   goes, for the leaf provers. ⏳ build that with the coordinator's work queue (e). (11-06) The COIN LOG too: a leaf
   loads `state_<h-1>.coinlog`, the whole log (mainnet: billions of leaves), so prep must write each block's leaf
   inputs (coin-log paths for its spends, the trie witness, the state) as one small bundle per block; the BIP30 tree is
   replayed from the block files (fine). Host only, no image change. **Design (11-07):** `leaf_<h>.bin`, written by
   libzync-prep (HAZYNC_PREP_LEAF_BUNDLES=<dir>) from its stores AT S(h-1), before it applies block h: S(h-1); the
   block's BIP30 witness (`BlockWitness.smt` + `in_smt_root`, what lz_smt_rebind computes; the BIP30 tree is NOT
   replayed by a leaf prover after all: at mainnet scale that means every block file below h); the coin-log witness
   (positions + siblings, as cl_prepare); the trie witness (TrieStore::witness). A leaf prover
   (`libzync-prove-range h h` with HAZYNC_LEAF_BUNDLE=<file>) then needs only the bundle and block h's JSON, no
   stores. Test: every block 1..200 proved (dev mode) as a leaf from its bundle; the guest checks every witness, and
   the joined root must equal the linear block 200. ✅ **built + tested 11-07** (host only, image bbf82800 unchanged; `leaf/RESULT_c14_leaf.txt`):
   mainnet 1..200: 200 bundles, 804 KB in all, largest 2,278 B (block 183), prep's states byte-identical to the proved
   run; leaves 198, 199 and 200 each proved with ONLY their bundle and their own block file (state = the linear one but
   `anchor_id`, as a base step); linear 197 + leaf 198 + leaf 199 + leaf 200 joined (3 joins, 6.8 s each in dev
   mode) = the linear block 200's journal exactly (edd7a213); prove-range 1..200 with HAZYNC_LEAF_CHECK: every block's
   coin-log, trie and BIP30 witnesses and S(h-1) byte-identical to its bundle's. Not yet: bundle bytes on mainnet-sized
   blocks (a prep run measures them), and the coordinator serving them (e).

**Mainnet prep plan (11-07; the long pole for any real-chain proof).** What it produces: the exact state before every
block (coin log, counting trie, BIP30 tree, block forest), then per-block leaf bundles (item 8). Sizes are ESTIMATES
until a prep run measures them: coin log ~2.4 B leaves × (32 B leaf + ~32 B of upper nodes) ≈ 150 GB, its position
index ~2.4 B × 40-byte slots at the index's load ≈ 100-200 GB, the trie (every spendable output ever, history kept;
258 B / coin measured on a SYNTHETIC chain) × ~3 B outputs ≈ 0.8 TB, a bitcoind block store ~0.7 TB. So a server
with ~2.5-3 TB NVMe, 32 GB+ RAM, 8+ cores; prep is single-threaded today. Steps:
1. a block source on the server: its own bitcoind (full blocks), and prep reading blocks over RPC instead of one
   block_<h>.json per height (JSON for the whole chain would be larger than the chain): host change, image unchanged;
   ✅ **built + tested 11-07** (be162dc, 22bf3d0): `fetch_block_rpc.py range <lo> <hi> <dir> --window N` (any
   `bitcoin-cli`, HAZYNC_RPC_CLI) feeds prep, which waits for each file (HAZYNC_PREP_WAIT_S) and with
   HAZYNC_PREP_CONSUME=1 deletes the files a save point covers; the BIP30 tree is snapshotted at each save point
   (`state_<h>.smt`), so a restart needs no block files below it. Test (mainnet 1..60 over ssh RPC, window 15, save
   every 10): prep killed at 35, restarted from 30 with only blocks 31+ on disk, every state byte-identical to the
   proved chain; 0 block files left at the end;
2. measure first: prep from genesis in disk mode (HAZYNC_PREP_SAVE_EVERY=1000, HAZYNC_PREP_CHECK against proved
   states where they exist), the rate and bytes per block recorded every 10 k blocks; the whole-chain time is
   estimated from those, not before; **measured 11-08:** 0 mismatches to 370 k; ~0.9 blocks/s to ~367 k, then
   0.18-0.28 once the trie (59 GB) passed RAM (62 GB) in the 2015 spam blocks. Cause: the bulk volume's WRITES, ~40 MB/s
   flat whatever the queue settings (wbt off: same), prep half its time in `balance_dirty_pages` (the service's
   MemoryHigh made it worse: lifted) and the rest in page faults; the disk arena reused freed 64-byte slots in random
   order, one dirty 4 KiB page each. Fixes: reuse them in address order (dbcc628, layout only): 0.50 blocks/s; then
   the trie, coin log and states moved to the root volume (vda: 40 k random-write IOPS; `/srv/bulk` stays the block
   and leaf store, symlinks keep the paths): **1.44 blocks/s** at 371.5-372.7 k, the process mostly on CPU again;
3. per-block leaf bundles (item 8) written as prep goes: ✅ built + tested 11-07 (HAZYNC_PREP_LEAF_BUNDLES);
4. a real recent block proved on real state over several GPUs, then the coordinator queue (e) for backfill.
**H7 DONE 11-07** (`h7/RESULT_h7.txt`, tag `libzync-c14` = source ce4e1e3): two container builds on the bridge box
agree, mainnet **863c9ce3**, signet **73b62530** (risc0 3.0.5; **superseded 11-11** by the 3.1.0 rebuild of the same
source at `7dfb05b`: mainnet **8888d59c**, signet **d4210dae**, header chain **2db34e12**, two builds agree,
`RESULT_risc0_3_1_gpu.txt`), pinned in `reproduce/LIBZYNC_IDS` and the verifier tables; the
container ELF behaves as c14 (962,000 aggregate 64,896,840 cycles, identical; mainnet 1..200 states identical but
`self_id`). ⚠ The laptop builds (bbf82800) differ by design (build paths): published proofs run the container ELF, and
a prep run whose leaves are to be JOINED must run a host embedding it (its states carry `self_id`, which the join's
link digest covers); the paper's single-leaf samples are unaffected (a leaf is a base step on an untrusted state).
**11-08:** the coordinator-box prep run switched to a host embedding 863c9ce3 at the save point 378,500 (7089c3d:
CONTINUE under another image re-stamps the saved state; test `prep/RESULT_restamp.txt`), so states and leaves from
378,501 on are joinable; 1..378,500 need a windowed re-prep under 863c9ce3 for the genesis join tree. (Historical:
`863c9ce3` was withdrawn 11-11. The board pass was restarted from genesis under `8888d59c`, SWITCH.md step 0. Which
image the paper prep run's states now carry is not recorded here.)

**Where it runs (user, 11-07): on the EXISTING bridge infrastructure, not a new server.** libzync prep is the
libzync BRIDGE: the same role as `hazync-bridge` (archive node beside bitcoind, resident state, one bundle per block),
its `leaf_<h>.bin` pushed by the same write-only `hazync-bundle-push` channel and served by the same coordinator
(`server.py`, HAZYNC_LIBZYNC=1). The open question is disk on the bridge box: the coin log + trie are ESTIMATED at
hundreds of GB to ~2.5-3 TB NVMe (not measured). Nothing here needs a GPU. **Looked 11-07:** the bridge box has 16 cores, 62 GB RAM of
which the original bridge holds 46 GB, 457 GB free; the COORDINATOR box has 16 cores, 59 GB RAM free, 3.5 TB free on
`/srv/bulk` and its own archive bitcoind (txindex, synced). So prep runs on the coordinator box (no deletions; room
for the trie in the page cache; leaf bundles where the coordinator serves them). The coordinator box also holds
349,263 original-Hazync bundles over heights 1..974,926: the CORE inputs for the same-card comparison. Disk fixes
first (13e395f): the position index follows the LIVE coins (~10-20 GB, was 100-200), coin-log levels 1..3 not
stored (~65 GB saved).

**Evidence the paper needs (user, 11-07), before writing:** (1) MAINNET block proofs on REAL chain state, across a
range of heights from genesis to tip (no modelled state): mainnet prep to tip on the bridge box, leaf bundles at the
sampled heights, GPU leaf proofs; (2) cycles first, then card time and dollars, **every measurement on ONE card
type**, with the original Hazync CORE guest re-measured on that same card for a direct ratio (replacing the chained
~8.2x). Proposed sample: ~10 heights spread genesis..tip by era (early, 2013-2016 growth, segwit 2017, taproot 2021,
recent full blocks), each proved by libzync from its leaf bundle and by CORE, on one PRO 6000 (cost to estimate per
block before renting).

**Coordinator work queue for backfill, H-B (e): design (11-07).** The coordinator already has the shape: width-1
block claims that expire by themselves (`claim`, advisory; `submit` accepts any valid proof), fold claims over
adjacent proven ranges (`fold_claim`, `GET /api/foldable`), a spine, and receipts that authenticate themselves
against one pinned image id (`expected_method_id`). Mapping:
- a block claim = a libzync LEAF: block h proved as a base step on prep's S(h-1);
- a fold claim = a JOIN (`libzync-join`, mode 18) of two adjacent proven ranges; the spine is the root of the tree,
  published when it covers genesis..tip.
What is new: (1) the coordinator serves prep's per-block leaf bundles (H-B 8: S(h-1), the coin-log paths, the trie
witness, the block), produced on the prep server; the coordinator stores and serves them and never computes
(decision_coordinator_serves_never_computes); (2) submit checks a libzync receipt: verifies against the FROZEN libzync
id (a libzync coordinator instance pins it in place of reproduce/METHOD_ID), and for a leaf requires
`anchor_id == link_digest(S(h-1))` and the tip = block h; for a join, the receipt verifies (the guest already enforced
the link) and covers the two ranges' union; (3) the worker side: `libzync-prove-range h h <bundle> <S(h-1)>` for a
leaf, `libzync-join` for a fold. Leaves on a modern block are ~9 min of PRO 6000 time (537 s); joins 6.4 s. ✅ **built + tested 11-07** (`coord/RESULT_c14_coord.txt`): a libzync host answers the original
coordinator's three host commands (`prover/host/src/lz_coord.rs`): `prove-range-bridge <h>` = a leaf from
`leaf_<h>.bin`, `fold-range` = a join, `verify-any <rcpt> <lo>` = the RANGE-OK line with link digests as tips (lo is
checked: the receipt's anchor_id must be the digest of genesis or of leaf_<lo>.bin's prev); `server.py` with
HAZYNC_LIBZYNC=1 serves `leaf_<h>.bin` (GENESIS_TIP = `host libzync-genesis-tip`); the worker stores an LZLEAF bundle;
`hazync-bundle-push` also pushes `leaf_*.bin`. Test: local coordinator + the stock worker, mainnet 1..4 in dev mode:
leaves 1, 2 submitted, `hazync run 3-4` (2 leaves + 1 join) accepted, frontier 4 from genesis; wrong start heights
refused; CI's 128 coordinator test commands pass. The leaf bundle now carries the block's JSON (LZLEAF02), so it is a
leaf prover's only input. Not yet: deployment (a libzync coordinator instance beside the original one), `scripts/check-deployment.sh` (still greps
`bundle_*.json`); the bundle-gap check reads `leaf_*.bin` with HAZYNC_LIBZYNC=1. ✅ **Deployed:** the libzync
coordinator ran beside the original on :8900 from 11-09 and became the public board on 2026-11-12
(`coordinator/deploy/libzync/SWITCH.md`); first run 1,287 blocks from genesis on 4× RTX PRO 6000 in 2 h 38 min.

Steps: (a) join mode + host `prove-anchored` / `join` commands (c10e11, before the freeze); (b) TEST: blocks 1..200 as
200 independent anchored proofs, joined: the root journal must be byte-identical to the linear run's block_200;
(c) negatives: gap, overlap / duplicate, swapped order, tampered anchor, other image, non-chain journal — all refused;
✅ (a)-(c) DONE 11-05 on c10e12 (a9dd2da5, dev mode): 200 leaves proved last-first 3 at a time, a balanced tree of 199
joins (8 levels, odd carries), root journal **byte-identical** to the linear block_200; join(linear 100, leaf 101) ==
linear 101; gap / swapped / twice / tampered anchor refused by the guest's link check, another image's receipt refused
by the host (`join/RESULT_c10e12.txt`). Native prep 1..200 (`host libzync-prep`) gives every state's journal
byte-identical to the proved run, trie in memory and on disk (`trie_store/RESULT_c10e12.txt`);
(d) in the GPU benchmark step (script ready 11-06: `gpu/gpu-join.sh`, `--mkbundle`; its DRY=1 dev-mode run passes on
the laptop, 8 leaves / 7 joins / 3 levels, root == linear block_8): a REAL (non-dev) join of joins (the dev-mode test shows byte-equivalence, not the
recursion: env::verify / assumption resolution and "succinct, joinable again" need real receipts), and MEASURE join
cycles / cost (the old fold was $0.0043, another guest), arity (2 / 4 / 8) and leaf size; ✅ **DONE 11-06 on c14**: 8 real
leaves on prep states, 7 joins over 3 levels, root identical to linear block 8 and verified; a join is one 2^20 segment,
6.4 s on a PRO 6000; root receipt 226 KB (arity and leaf size not yet measured); (e) after the freeze: the coordinator's work queue (prep ahead, leaves in any order, joins as children
land, resume points). ✅ running on the public board since 11-12 (claims = leaves, folds = joins, the
genesis-anchored spine).

## I. Gaps beyond the core (11-04)

What SwiftSync + Utreexo + libzync still do not solve, and how each is closed. None changes the core image unless a
test finds a bug.

| # | gap | how | status |
|---|---|---|---|
| I1 | trust in our guest | Core's own test vectors (`script_tests`, `tx_valid`, `tx_invalid`) through the guest; fuzzing; extend the rig | **harness built 11-05 (c909841)**: guest mode 10, `scripts/libzync/core_vectors.py` (1,420 base cases = Core 28's counts, 22,666 with flag variants), `host core-vectors`; **c10e10, base set, libsecp path: 1,420 / 1,420 agree with Core, 0 differ** (exact ScriptError for script_tests; 68 by a guest abort where Core throws and catches inside EvalScript and fails the script: the guest has no C++ unwinding, both reject; `core_vectors/RESULT_c10e10_base.txt`). **No unwinding, by decision (user, 11-05):** every catch the guest can reach rejects the block (EvalScript, signet), pinned in guest build.rs `REVIEWED_CATCHES` (the build walks the include graph and fails on any change), the host reports an abort as `BLOCK REFUSED`, CONSENSUS_RULES.md states it. **c10e11, the BATCHED signature path: honest hints 1,420 / 1,420 (exact ScriptError too); FORGED hints (every verify claimed valid with an R that passes the per-signature check; opposite parity; every other verify) 0 Core-invalid cases pass** (67-80 Core-valid cases fail under a lying host: liveness only; x = r + n is infeasible for real r, effectively untested) (`core_vectors/RESULT_c10e11_sig6.txt`). **c10e12, every flag variant (22,666 cases), libsecp path: 22,666 / 22,666 agree, 0 differ** (679 by a guest abort where Core catches) (`core_vectors/RESULT_c10e12_variants.txt`). ⏳ fuzzing; extend the rig |
| I2 | trust in our guest + verifiers | external audit and a bug bounty before anyone relies on proofs for money | after freeze |
| I3 | trust in the zkVM | pin one risc0 version and track its audits (11-07: the frozen guest's risc0-zkvm 3.0.5 / platform 2.2.3 / zkos-v1compat 2.2.3 are past the fixes for CVE-2025-61588 and CVE-2025-52484, the critical host-input and circuit advisories; **11-11: moved to risc0-zkvm 3.1.0**, which fixes soundness bugs in the rv32im circuit, `sys_read` range checks, the recursion circuit's `BIT_AND_ELEM`, Groth16 claim handling and the Poseidon S-box; 3.1.0 verifiers refuse 3.0.x receipts; released as v0.24.0, v0.23.0 withdrawn); optional: prove periodic checkpoints on a second zkVM (e.g. SP1), so one prover bug cannot forge alone (~2x cost, checkpoints only) | 3.1.0 pinned (v0.24.0); second zkVM: later, user's call |
| I4 | data availability | the blocks are the data: publish coin log / tree snapshots (verifiable against proven roots, any untrusted mirror works) + a "rebuild from my own node" tool | after freeze |
| I5 | proving cost | the 10x target (E1); one proof per block shared by every user; zyncAPP revenue pays (payment is the rate limiter); speculative pre-proving (D1) moves work off the critical path | ongoing |
| I6 | not consensus | Ghost pool commits the proven roots in its coinbase (no fork; miner attestation on chain); wallet / exchange adoption via the verifiers; a soft fork is a community process for much later | I6a Ghost coinbase: after freeze (Ghost-side, user's call) |
| I7 | tip latency | speculative chunks (D1, 7.5% waste measured), zeroconf zyncAPP (D5), more cards per block; trim the tree's +14-29 s execution | ongoing |
| I8 | privacy of queries | whole-snapshot download and local queries; batched / padded queries; private information retrieval later | later |

## J. SwiftSync: what Hazync can offer Ruben (11-05)

An offer, not a dependency: SwiftSync stays as it is, Hazync only removes its worst failure mode (a bad hints file is
found only at the END of a sync). Proofs of concept from design B are kept at tag `archive/swiftsync-design-b`
(806d9e1, the last commit before E2 removed them). The hand-over is a one-page note the user edits and sends.

| # | idea | what exists | status |
|---|---|---|---|
| J1 | **SwiftSync zAPP: a proven hints file for ANY height H.** The core already proves every block and commits the coin log (and the block forest, each block's coin state); a zAPP walks the log as it stood at H, emits one bit per output (unspent at H), commits (H, block hash at H, sha256(hints)). Anyone can serve the file; a node checks the receipt against a Hazync tip proof BEFORE syncing, then still validates every block itself (the proof buys liveness, not trust). A new H is a new zAPP run, not a new checkpoint scheme: the "moving H" answer | design only | candidate, after the freeze. First: (a) the hints index EVERY output in block order, the log holds only coins that outlived their block and are not OP_RETURN (all 0 bits): check a coin leaf (or the tx log, D) gives each survivor's position in its block and each block's output count; (b) MEASURE the cost (every log leaf up to H; parallel by range + join; once per H) |
| J2 | **proven hints file inside the proof (design B).** Ruben's check on a proof: `ss_set == 0`, `height == H`, `ss_hints == sha256(file)` | built + GPU-proved: signet 2501..2600 verified on GPU; verifiers (cli / ffi / wasm) took the hints file | retired from the core in E2 (tag); J1 is its zAPP form |
| J3 | **the coin log IS a hints file for every H**: leaves in birth order, a spend flips its leaf in place, so a leaf's state at H is that output's hint bit; one structure serves hints for any H as it moves with the tip | the coin log (COINLOG.md), live in the core | idea for Ruben; the basis of J1 |
| J4 | **BIP30 done exactly**: only the transactions that can ever repeat are tracked (pre-BIP34 coinbases indicating a later height, and every tx whose inputs are all such coins), with live counts: a repeat is refused while it has unspent outputs, allowed once it has none, as Core. Measured on mainnet to 974,111: 173 such transactions for indicated heights under 100M, 22 outputs unspent, 1 after block 300,000; the grandfathered 91,842 / 91,880 overwrites. Any hints-based sync must get these right | `coinbase-smt/src/bip30.rs`, 29188b5, rig 83/84 agree with Core 28.0 | data + method to share |
| J5 | **hints-format hygiene**: canonical encoding rules, a Python reference, malformed-file vectors (flipped bit, non-canonical size, trailing byte, bad version), a hint-hash definition both sides compute the same way | at the tag: `scripts/libzync/ss_hints_ref.py`, `swiftsync-verify/vectors/bad_*.hints`, `libzync/src/ss_hints.rs` | ready to share as is |
| J6 | **SwiftSync + moving H + Utreexo**: SwiftSync to reach H fast, a Utreexo accumulator to carry the tip with no UTXO set; J3 bridges the two. The coherent end state for SwiftSync (NOT for Hazync, whose design stays the coin log + trie) | design notes: `experimental/libzync-bench/HANDOVER_DESIGN.md` (option B) | idea for Ruben |
| J7 | the note: one page, J1-J6, what each is, what was measured, where the code is | not written | after the freeze, user edits and sends |

## K. zAPP ideas for the relaunch (11-05)

Hazync relaunches on libzync (the new core library) with the zAPP ecosystem around it. Cheap, visible zAPPs first: each
is a marketing win that shows the core working.

| # | zAPP | what it proves | cost | status |
|---|---|---|---|---|
| K1 | **header chain** | the most-work chain of valid proof-of-work headers from genesis: double SHA-256 under target, nBits and the 2,016-block retarget, median-time-past, chainwork. SPV-level (no transactions, coins or scripts); the core already runs these checks inside every block proof. Kept at the tip at a fixed few seconds a block (one header ~3 k cycles, far below the smallest segment) | ESTIMATED, not measured: ~2-3 k cycles a header (3 compressions: 283 cycles each on words, 1,105 per 65-byte `sha2` hash, measured; the rest small) => ~2-3 B cycles for ~975 k headers => ~40-60 min on one RTX PRO 6000 (1.2 s per M cycles, measured) ~$2, or ~1.5-2.2 h on an A40 ~$1; minutes on 16 cards. About the cost of 5-7 full modern blocks | **BUILT 11-05** (zyncapps guest `headers`, `zyncapp headers-exec`): Core's header rules (prev link, CheckProofOfWork, CalculateNextWorkRequired, time-too-old, BIP34/66/65 versions, GetBlockProof work; target and work computed once per nBits). Executed on REAL mainnet headers from the node: 0..2,100 (through the first retarget) end hash and chainwork **identical to the node's**; tampered nonce / time / bits / prev refused (high-hash, time-too-old, bad-diffbits, link). **MEASURED: 2,427 cycles per header** (genesis..2,100). Whole chain ~975 k headers = ~2.4 B cycles; at the MEASURED core proving rate (1.2 s per M cycles, PRO 6000) ~47 min ~ $2, or A40 ~1.8 h ~ $1 (proving time an estimate from cycles until proved). Recent range 967,681..971,720 (4,040 headers, real retargets at 969,696 and 971,712: nBits 17021ec5 -> 17021ef0 -> ...): end hash and chainwork **identical to the node's**, 2,421 cycles per header. Re-checked on the rebuilt guest (11-05, run from genesis starts empty): both ranges identical to the node again (2,425 / 2,419 cycles per header). ⏳ a proved run; joins of runs (state digest in, state out) for the tip. Launch line: "the whole Bitcoin header chain, proved, for about $2". ✅ **PROVED (N3):** genesis..975,278 in one receipt under `2db34e12` (risc0 3.1.0): 59.4 min on one RTX PRO 6000 (~$2.50), 2,350,725,289 cycles, 224,138 bytes; WASM verifier 39 ms on a laptop under Node (`RESULT_risc0_3_1_gpu.txt`; the 3.0.5 proof under `4d04d6bf`, 57.2 min, is retired). At the tip: `hazync-headers-follow` chains one CPU-proved run base+1..tip to that base (`coordinator/deploy/headers/`) |
| K2 | SwiftSync hints for any H | see J1 | see J1 | candidate |

## L. Business model for zAPPs (11-05; reasoning, not measured demand)

The code is open source, so it is not the moat. Open SDK (anyone builds zAPPs) + a curated registry + our paid proving
network. A marketplace grows out of the registry and the proving service; it is not built first, and builders are
not gated (that cannot be enforced, and slows the ecosystem).

| # | lever | why it pays | revenue |
|---|---|---|---|
| L1 | **registry of reviewed zAPP image ids** | a zAPP proof means nothing unless the verifier trusts its image id; whoever publishes the reviewed, reproducibly built ids is the gatekeeper wallets / exchanges rely on (the app-store position) | review + listing fees for third-party zAPPs |
| L2 | **proving network + live state** | every zAPP needs the proven tip and its state (coin log, trie, block forest) kept current per block, plus GPUs: cheap to write a zAPP, expensive to run this | pay per proof, subscriptions for tip-following; a take rate on third-party zAPPs proved on it. Payment is the rate limiter (as the coordinator) |
| L3 | **first-party zAPPs for buyers with budgets** | proof of reserves (exchanges, custodians: coins unspent at the tip, summing to X) — the clearest paying market; Bitcoin state for bridges / L2s (BitVM-style bridges, sidechains, rollups) — likely the most lucrative; instant IBD / UTXO-set checks (node and wallet vendors); wallet light clients (balance, history, absence: volume, low price) | per proof / per query / contracts |
| L4 | marketing zAPPs | header chain (K1), SwiftSync hints (J1): cheap, visible, not revenue | none directly |

Order: freeze the core -> ship 2-3 first-party zAPPs (header chain for the launch, then proof of reserves and coin /
balance facts) -> open the SDK with the reviewed-id registry -> talk to 2-3 exchanges and one bridge team BEFORE
building more (their answers set the pricing). Cheapest real test of demand: ask them.

## M. Cost after the freeze (11-05; candidates, none measured yet)

The dominant cost is verifying every signature in Bitcoin's history; what is left is mostly levers on how that and the
memory around it are paid for. Each is measured on the cost table (9 modern blocks + 962,000) before it lands, and any
change after the freeze ships as a new image through the handover.

| # | lever | why it might pay | size |
|---|---|---|---|
| M1 | "verify, don't compute" in more places: the host supplies an answer, the guest checks a cheap certificate (as liftx, batch signatures, ECMH points, trie witnesses already do): divisions (q(t+1) <= ~t < (q+1)(t+1)), sorts (check a permutation), anything the host already knows | safe by construction (a wrong hint fails); local | small, per site |
| M2 | zkVM memory, not arithmetic: risc0 charges cycles for paging; libsecp's precomputed tables are tuned for CPUs (big tables, fewer ops). Smaller tables / other window sizes may win in the zkVM | contained: a few table sizes on 962,000 | small experiment |
| M3 | padding: proving pays padded (power-of-two) segments, not raw cycles; better chunk packing or a per-chunk segment size | no change to what is proved | small-medium |
| M4 | bigger signature batches: the batch check (Pippenger) grows sub-linearly, so batching across chunks or blocks amortises more; moves the batch up a level (aggregate or its own receipt) | measure the curve first | medium, design change |
| M5 | a specialised signature prover: a block's signatures proved in a system built for curve arithmetic (custom gates, GKR-style batch ECDSA), its proof verified inside the zkVM | the big ZK teams' route to large signature speedups; guess 2-5x on the signature share, UNMEASURED; a second proof system to audit | weeks; research project, after the GPU benchmarks say what signatures cost in dollars |

Not levers: skipping signatures (assumevalid, ruled out: every block is proved); replacing Core's interpreter (<1% of
cycles, measured); a risc0 upgrade for new accelerators (ruled out; the 11-11 move to 3.1.0 was for soundness, I3,
and kept the cycles the same).

## N. Launch (11-10; status 11-12)

What goes out together, in order. Each item is the operator's call to ship.

| # | item | status |
|---|---|---|
| N1 | release v0.23.0 (#692): the libzync host, verifier and Windows CUDA host | ✅ **done, then replaced:** v0.23.0 (risc0 3.0.5, `863c9ce3`) is **withdrawn**; **v0.24.0** (2026-11-12, risc0 3.1.0, `8888d59c` / `d4210dae` / `2db34e12`, 12 assets; `docs/RELEASE_NOTES_v0.24.0.md`) is the release |
| N2 | the board switch (`coordinator/deploy/libzync/SWITCH.md`; #676, #684) | ✅ **done 2026-11-12**: steps 0-6, and step 7 (website). Restarted from genesis under `8888d59c`; v1's board (frontier 144,769) kept read-only as the record; 13 contributors carried over |
| N3 | **the header-chain zAPP (K1)**, proved from genesis to the tip and checkable on hazync.org | ✅ **done**, steps below |
| N4 | the paper | draft 2, private; written with the operator |

N3, step by step (all done):

1. Execute the whole chain from genesis against our node: end hash and chainwork must match. ✅
   (`experimental/libzync-bench/headers/RESULT_exec_whole_chain.txt`)
2. `zyncapp headers-prove` (a succinct receipt) and a verifier: `swiftsync-verify headers`, and `verify_headers` in
   the WebAssembly build. The verdict follows the core's rule: OK once the chainwork reaches Core's minimum, DEMO below. ✅
3. Pin the image: a reproducible build gives the same id bit for bit, as for the core (`reproduce/`). ✅ `2db34e12`
   (`reproduce/LIBZYNC_IDS`)
4. Following the tip: a run starts from the state the previous one ended in (the guest commits both), so new headers
   are a short run chained to the last one; the verifier checks the chain of runs. ✅ as base + one run:
   `hazync-headers-follow` on the board box re-proves base+1..tip on CPU every 10 minutes and publishes it only after
   `swiftsync-verify headers` accepts base + run (`coordinator/deploy/headers/README.md`). Re-basing is open (O6).
5. A page on hazync.org that verifies the proof in the browser. ✅ `/headers/`
6. One proving run on a GPU, the operator's trigger. ✅ 59.4 min on one RTX PRO 6000, ~$2.50 (11-11, 3.1.0)

No prep is needed: headers carry no coin state.

## O. Open after the 12 Nov run

What the switch and the first v2 proving run surfaced. None changes the core image.

| # | item | what is known | status |
|---|---|---|---|
| O1 | **following the tip with libzync** | the board proves from genesis, paced by the prep pass (`libzync-board-prep`); there is no v2 bridge and nothing proves new blocks as they arrive | open |
| O2 | **Coin check on the board's latest proof** | hazync.org `/zapp/` checks trie facts against the fixed mainnet 1..200 proof; it should read the board's newest genesis-anchored proof and facts at its height | **built 11-12**: prep writes `history.bin` (`HAZYNC_PREP_HISTORY=1`; byte-identical to a proving run's, 0..200 in one run and resumed); `hazync-lz-zapp-follow` brings history and trie to the spine's height; the coordinator serves `/api/zync/proof`; the page loads it. Live once deployed |
| O3 | **Litestream for the v2 ledger** | Litestream is set up for v1's database (`coordinator/deploy/litestream.yml.example`, `dropins/litestream-hazync.conf`); the v2 coordinator's database under `/var/lib/hazync-lz` has no continuous replica (only the retargeted `hazync-coordinator-backup` snapshots, kept on the box, and the `hazync-lz-offsite-*` proof and spine copies; SWITCH.md step 3) | **done 11-12**: `coordinator-lz-server1` |
| O4 | **wasm toolchain pin** | the browser verifier is byte-reproducible only with the same rustc: Rust 1.99.0 gives the published `446fdaeb…`, 1.97.1 other bytes; v0.24.0 was built with `RUSTUP_TOOLCHAIN=1.99.0` set by hand | **done 11-12**: `build.sh` pins Rust 1.99.0 (verified on a box whose default is 1.97.1) |
| O5 | **sm_120 native kernels** | the release CUDA host carries native kernels for sm_80/86/89/90/100 + compute_100 PTX; RTX PRO 6000 (sm_120) JIT-compiles ~10 min on a fresh machine's first prove (cached afterwards) | **in the build 11-12** (`build-release.sh` NVCC_FLAGS): a test build carries sm_120 SASS in all 52 fatbins (the v0.24.0 binary in none), still 8888d59c, 422 MB against 391 MB; ships with the next release, first run on a real sm_120 card to confirm no JIT |
| O6 | **headers-follow re-basing** | each run re-proves base+1..tip in full: at 250 headers it peaked at 9.8 GB RAM (MemoryMax raised from 6G to 24G after the 6G cap OOM-killed it from 11-11 19:10); prove a fresh GPU base (`--set-base`) before the run grows too long | open |
| O7 | per-key open-claim cap | `CLAIM_OPEN_MAX` defaults to 4: one key running more than 4 workers starves itself (measured 11-12); restarting workers mid-run orphans their claims, and an orphaned frontier claim stalls the frontier until it lapses (up to `CLAIM_TTL`, 60 min) | known; operator guidance. `run-workers.sh --drain` (11-12) lets workers finish their blocks instead of orphaning them |
| O8 | **verifier default for v2 receipts** | without `--product`, `hazync-verify` reads a v2 journal the v1 way and reports "UTXO set at H: N coins (Utreexo roots)"; the verdict is right, the description is v1's. Found by the stranger test (11-12); the site and CONTRIBUTING now pass `--network mainnet --product` | open: detect the product image from the journal or default to it, next release |
| O9 | **independent review (Fable, 2026-11-12)** | four reviewers on 713ba28: guest soundness, verifiers + website, board, host / zyncAPPs / release. Guest: no critical, high or medium finding (join, checkpoint, coin log, trie, batch ECDSA traced sound). Board: a receipt was checked only where it STARTS, so a block mined on the board's real state (difficulty 1 early on) was accepted as Bitcoin's, and a longer such chain from genesis could replace the spine; three junk POSTs cancelled another worker's claim; free keys let one IP hold every low block; `verify-any` did not refuse dev-mode receipts. Website: Coin check showed a genuine fact about ANOTHER script under the question's heading. Host: the zapp follower would have published a disk-trie marker zync-serve cannot load. Release: the signature covers whatever is on the release, the CI gate passes on any green workflow, the Windows host builds from mutable caches | **fixed 11-12:** `verify-any --check-end` (the end state must be the board's own; the board refuses a line without `end=board`; spine verified once), failures count only from the live claimant with a valid signature, `CLAIM_NEW_IP_MAX` (8), Fake / dev-mode refused (#705); Coin check binds the fact to the question (web #50); zapp follower keeps its trie in memory and rolls back if zync-serve does not stay up (#705). **Open, before the next release:** release signing and CI gate, Windows build inputs, unpinned build fetches, prep restart order after a crash, sponsor invoice holds, verify-before-cheap-checks, `Content-Length: -1` if :8899 is reachable directly |

## Image ids built (11-03, unit tests + compiles only)

⚠ **Historical.** Every id in this table is a laptop build on risc0 3.0.5 and is refused by today's verifiers. The
published ids are in `reproduce/LIBZYNC_IDS`: mainnet `8888d59c`, signet `d4210dae`, header chain `2db34e12`
(risc0 3.1.0). Other retired ids met in this file: `bbf82800` (c14 laptop), `863c9ce3` / `73b62530` (v0.23.0
container, 3.0.5, withdrawn), `9f469930` / `067822b9` (round-8 images, 10-31), `4d04d6bf` (header chain, 3.0.5).

| image | id | note |
|---|---|---|
| c8m (checkpoint) | 9d9adb54 | superseded |
| c9tx (coin log + txlog) | df86c392 | |
| c9m (coin log) | 0d8cac45 | ⚠ built while a zyncAPP build held part of target/riscv-guest: rebuild from clean before the test pass |
| zyncAPP coin | 3f503000 (v2, 11-07; v1 26e09637) | |
| zyncAPP zeroconf | a61caf76 | |
| c10b / c10o / c10s / c10os (E1 measurement) | e7514dc8 / 629005ae / 3c948172 / 95df867e | tree state from the witness; cost only |
| c10h / c10oh (E1 history script tree) | b704daf3 / cba70157 | ⚠ undercounted (within-block coins); superseded by c10h2 |
| c10h2 / c10t (corrected history tree / counting trie) | ab3f1ac1 / a87a541e | tree state from the witness; cost only |
| c10t2 / c10t3 / c10t4 (trie trims 1-3) | 00ec71b3 / 73158901 / 58bab460 | tree state from the witness; cost only |
| c10t5 (trie, final measured) | c7c1463e | tree state from the witness; cost only |
