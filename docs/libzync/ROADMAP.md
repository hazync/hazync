# libzync: roadmap to the freeze and beyond (11-03)

Everything built, discussed or promised so far, in build order. Status words: **done**, **building**, **next**,
**experiment** (built behind a flag, measured, then decided).

⚠ **Testing policy until phase H:** unit tests and compiles only. No long suites (rig, handover, signet from genesis)
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
| B5 | zyncAPP `coin` guest (prover/zyncapps): unspent/spent at the core tip, core receipt verified in the zkVM; image 26e09637 | done |

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
| E2 | integrate the counting trie (c10t5 design) into the core: trie root in ChainState bound proof to proof (not from the witness); the trie updated in its OWN receipt in parallel with the chunks and verified inside the core proof (split measured: serial aggregate 192 M -> 99 M, whole-block cycles +2.4%; pinned in the core image like the chunk program); host trie store; absence / history / balance verifiers (cli, wasm, C ABI); zync-serve endpoints | next |
| E2b | instant IBD: the chain state's ECMH total tracks the LIVE coin set (`utxohash`), so a new node verifies a downloaded UTXO set (~10 GB) against the proof. Measured +4.56% padded over 9 modern blocks (+1.2-1.8% on recent blocks). **CHOSEN 11-04 (user)**; part of E2 | chosen |
| E2c | consolidation (after E2a batches 2-3, before E2 integration and the reviews): ONE product configuration (coin log + counting trie as a parallel receipt + live-coin hash; signet / regtest / txlog kept); delete the dead experiments from the guest (otree / stree / htree / itree, design B + hints file, msm / field-bench / liftx-accel / tx-cycles, trie inside-vs-split switch); split `libzync` into the core library (coin log, trie update, facts, wire) and tooling / bench code kept out of anything that ships; break host main.rs into modules (prover, verifier, tools; benches out of the main binary). Cost table + unit tests green after each step | **done 11-05** (10161bf, 361e4f5, b26ca4f, host split): product image 16d9409b; every step's aggregate journal identical to batch 2's apart from the image-id fields. Left for E2: the wire's hint-bit field, the chain state's hint hash, libzync-verify's hints-file branch. ⚠ pre-existing host test failure smt_bridge::a_fully_spent_coinbase_can_be_duplicated_with_no_special_case (BIP30 coinbase duplication in the bridge SMT) — review rounds |
| E2a | cycle hunt across all core units (chunk, aggregate, trie guest): profiles in `~/libzync-gpu/e1/prof`, three read-only agents, each candidate measured on the cost table before it lands. Batch 1 (whole blocks -0.8 to -12.5%), batch 2 (aggregate -28.5%, whole -6.4%), batch 3 (trie guest -11.3%) | done 11-04 (more trie work possible later) |

## F. main branch

| # | item | status |
|---|---|---|
| F1 | port the `is_base` forgery fix and the BIP68 fix to `main` | draft PR #668 (merge + deploy: user) |

## G. Docs

| # | item | status |
|---|---|---|
| G1 | spec text for each item as it lands (SWIFTSYNC_PROOFS, COINLOG, ZYNCAPPS, txproofs) | with each item |

## H. Path to the core freeze (the user's gates, in order)

1. ~~E1 tree decision~~ — **done 11-04: the counting trie (E1c) is the path forward.**
2. **E2a** cycle-hunt batches 2-3, then **E2c** consolidation (one product configuration, dead experiments deleted, library split), then **E2** integration (trie as a parallel receipt inside the core proof, live-coin hash) — moves the image.
3. Several cold review rounds on everything since `00b7ac2` (feedback: reviews before any freeze).
4. One full test pass on the final image: rig vs Core, Core's own test vectors (I1), handover, mainnet 1..200, signet
   from genesis, edge blocks, cost table.
5. **GPU proving benchmarks** on the final image (user's trigger).
6. **Freeze: ask the user.**
7. After freeze: container ids built twice, LIBZYNC_IDS + verifier IMAGES tables (cli/ffi/wasm), vectors regenerated
   from GPU run A, proposal page refresh. A full mainnet proof is the user's trigger.

## I. Gaps beyond the core (11-04)

What SwiftSync + Utreexo + libzync still do not solve, and how each is closed. None changes the core image unless a
test finds a bug.

| # | gap | how | status |
|---|---|---|---|
| I1 | trust in our guest | Core's own test vectors (`script_tests`, `tx_valid`, `tx_invalid`) through the guest; fuzzing; extend the rig | next (before freeze, part of H4) |
| I2 | trust in our guest + verifiers | external audit and a bug bounty before anyone relies on proofs for money | after freeze |
| I3 | trust in the zkVM | pin one risc0 version and track its audits; optional: prove periodic checkpoints on a second zkVM (e.g. SP1), so one prover bug cannot forge alone (~2x cost, checkpoints only) | later; user's call |
| I4 | data availability | the blocks are the data: publish coin log / tree snapshots (verifiable against proven roots, any untrusted mirror works) + a "rebuild from my own node" tool | after freeze |
| I5 | proving cost | the 10x target (E1); one proof per block shared by every user; zyncAPP revenue pays (payment is the rate limiter); speculative pre-proving (D1) moves work off the critical path | ongoing |
| I6 | not consensus | Ghost pool commits the proven roots in its coinbase (no fork; miner attestation on chain); wallet / exchange adoption via the verifiers; a soft fork is a community process for much later | I6a Ghost coinbase: after freeze (Ghost-side, user's call) |
| I7 | tip latency | speculative chunks (D1, 7.5% waste measured), zeroconf zyncAPP (D5), more cards per block; trim the tree's +14-29 s execution | ongoing |
| I8 | privacy of queries | whole-snapshot download and local queries; batched / padded queries; private information retrieval later | later |

## Image ids built (11-03, unit tests + compiles only)

| image | id | note |
|---|---|---|
| c8m (checkpoint) | 9d9adb54 | superseded |
| c9tx (coin log + txlog) | df86c392 | |
| c9m (coin log) | 0d8cac45 | ⚠ built while a zyncAPP build held part of target/riscv-guest: rebuild from clean before the test pass |
| zyncAPP coin | 26e09637 | |
| zyncAPP zeroconf | a61caf76 | |
| c10b / c10o / c10s / c10os (E1 measurement) | e7514dc8 / 629005ae / 3c948172 / 95df867e | tree state from the witness; cost only |
| c10h / c10oh (E1 history script tree) | b704daf3 / cba70157 | ⚠ undercounted (within-block coins); superseded by c10h2 |
| c10h2 / c10t (corrected history tree / counting trie) | ab3f1ac1 / a87a541e | tree state from the witness; cost only |
| c10t2 / c10t3 / c10t4 (trie trims 1-3) | 00ec71b3 / 73158901 / 58bab460 | tree state from the witness; cost only |
| c10t5 (trie, final measured) | c7c1463e | tree state from the witness; cost only |
