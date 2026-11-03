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
| D3 | Phase B (context-free validity records) only if Phase A waste is large | after D4 measures |
| D4 | `fetch_template_rpc.py` (template -> speculative fixture); `template_waste.py` measuring re-prove waste | built; measuring |
| D5 | zyncAPP `zeroconf` guest: speculative chunk record + every input unspent at the tip | building |
| D6 | Ghost proof-ready blocks: the pool runs D4 on its own template (Ghost-side integration; Ghost infra is separate) | after D4 measures |

## E. Proof of absence

| # | item | status |
|---|---|---|
| E1 | keyed tree for absence: measured 283 cycles/hash => ~+40% on 962,000; recommended as a separate recursive zyncAPP chain beside the core, not in it (absence/README.md) | design; needs your call |

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

## Image ids built (11-03, unit tests + compiles only)

| image | id | note |
|---|---|---|
| c8m (checkpoint) | 9d9adb54 | superseded |
| c9tx (coin log + txlog) | df86c392 | |
| c9m (coin log) | 0d8cac45 | ⚠ built while a zyncAPP build held part of target/riscv-guest: rebuild from clean before the test pass |
| zyncAPP coin | 26e09637 | |
| zyncAPP zeroconf | a61caf76 | |
