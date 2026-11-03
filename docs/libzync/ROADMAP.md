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
| B4 | `swiftsync-verify --coin / --journey` (wasm/ffi exposure: next) | done (CLI) |
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
| E1 | keyed tree for absence: estimated +14% to +74% per block (absence/README.md); build behind a flag and measure late; script-keyed variant is the one proof of reserves needs | estimated |

## F. main branch

| # | item | status |
|---|---|---|
| F1 | port the `is_base` forgery fix and the BIP68 fix to `main` | draft PR #668 (merge + deploy: user) |

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
