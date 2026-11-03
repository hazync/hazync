# libzync: roadmap to the freeze and beyond (11-03)

Everything built, discussed or promised so far, in build order. Status words: **done**, **building**, **next**,
**experiment** (built behind a flag, measured, then decided).

⚠ **Testing policy until phase H:** unit tests and compiles only. No long suites (rig, handover, signet from genesis)
until everything is built and reviewed; then one full test pass on the final image.

## A. Chain identity

| # | item | status |
|---|---|---|
| A1 | guest pins one checkpoint (`checkpoint.rs`: mainnet 970,000, signet 325,000); verifiers give DEMO below its height | building (image c8) |
| A2 | `--expect` / `--mempool` / `--mempool-url` in both verifiers, refuse mistyped flags, unit tests | done (`3785f56`, `8e47047`) |
| A3 | host verifier `--bitcoin-cli` (as `swiftsync-verify`) | next |

Minimum work stays a verifier check: every proof below the tip is low-work by construction, so the guest cannot
refuse it.

## B. zyncAPP coin facts (ZYNCAPPS.md, COINLOG.md §6)

| # | item | status |
|---|---|---|
| B1 | `libzync::facts`: `check_core_statement` + the coin-fact checks (unspent at tip, spent by (h, tx, vin), state at height h through the block forest), no_std, shared by native verifiers and zyncAPP guests | next |
| B2 | host: produce the paths (`coinlog-path`, `block-path`) from the saved log and block forest | next |
| B3 | coin history: a transaction's Merkle branch + block path, chained backwards through inputs and forwards through spender records | next |
| B4 | `swiftsync-verify coin …` CLI over B1 (and the wasm/ffi builds) | next |
| B5 | the first zyncAPP guest: "coin C at its position is unspent/spent at the proof's tip", verifying a core receipt inside the zkVM | next |

## C. Production host

| # | item | status |
|---|---|---|
| C1 | disk-backed coin-log store (mainnet ≈ 2.4 bn leaves; today in memory) | next |
| C2 | path service: an HTTP endpoint serving B2's paths for a tip receipt (payment hook later) | next |

## D. Transaction log (txproofs/README.md)

| # | item | status |
|---|---|---|
| D1 | validity-batch guest: script-checks a batch of mempool transactions, commits one record each | experiment |
| D2 | log guest: appends batch records to a stump forest, rolling receipt | experiment |
| D3 | aggregate `txlog` feature: open each block transaction's record instead of re-proving it; unseen txs in a chunk | experiment |
| D4 | host mempool pipeline (batch every N s; RBF/eviction waste ≈ 1.3 % measured) | experiment |
| D5 | zero-conf zyncAPP (record in the tx log + unspent inputs in the coin log at the tip) | after D3 |
| D6 | Ghost proof-ready blocks (pool pre-proves its template) | after D4 |

## E. Proof of absence

| # | item | status |
|---|---|---|
| E1 | keyed tree (outpoint-keyed sparse tree beside the coin log): "X never existed / is not live" | experiment; measure aggregate cost first, keep only if it pays |

## F. main branch

| # | item | status |
|---|---|---|
| F1 | port the `is_base` forgery fix and the BIP68 fix to `main` as a PR (merging and deploying stay the user's call) | next |

## G. Docs

| # | item | status |
|---|---|---|
| G1 | spec text for each item as it lands (SWIFTSYNC_PROOFS, COINLOG, ZYNCAPPS, txproofs) | with each item |

## H. Review, test, freeze (the user's gates)

1. Several cold review rounds on everything since `00b7ac2` (feedback: reviews before any freeze).
2. One full test pass on the final image: rig vs Core, handover, mainnet 1..200, signet from genesis, edge blocks, cost table.
3. **Freeze: ask the user.**
4. After freeze: container ids built twice, LIBZYNC_IDS + verifier IMAGES tables (cli/ffi/wasm), vectors regenerated
   from GPU run A, proposal page refresh. GPU runs and a full mainnet proof are the user's trigger.
