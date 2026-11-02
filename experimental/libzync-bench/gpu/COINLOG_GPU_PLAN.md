# GPU test plan: the coin-log image

Status: ready to run once the guest is frozen. Spends money: the user starts every GPU run.

## 0. Before any GPU run

1. **Freeze check-in with the user**, after `handover/retest_c4.sh` passes in full (rig, handover, mainnet 1..200,
   signet 1..2600, cost table).
2. **Reproducible ids, built twice:** `docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .` on two
   clean builds; both must print the same `libzync-coinlog` mainnet and signet lines (`reproduce/libzync-ids.sh`,
   coin log by default). ⚠ It builds with `-j$(nproc)`: on the 8 GB laptop run it with nothing else building.
3. Write those lines to `reproduce/LIBZYNC_IDS`; add the two images to `swiftsync-verify`'s `IMAGES` with profile
   `PROFILE_LIBZYNC | PROFILE_COINLOG` (`| PROFILE_SIGNET`), the CLI/FFI/wasm tables, and the proposal page.
4. RunPod balance: top up for the runs chosen below (it was $1.47 on 11-02).

## 1. Runs, in order of value

| # | run | script | what it shows | expected |
|---|---|---|---|---|
| A | **signet 1..2600 from genesis**, 1× RTX PRO 6000 | `signet/genesis/mkbundle.sh 2600` then `gpu_run.sh 2600` (COINLOG=1 default) | a real, genesis-anchored coin-log chain proof; `libzync-verify <rcpt> -` says **OK** (not DEMO); its state equals `coinlog_ref.py` and `block_forest_ref.py` | the 2501..2600 design-B run cost $1.39 per 100 blocks; early signet blocks are smaller. A deadline (`DEADLINE_MIN`) and checkpoints (receipt + `state_<h>.coinlog`) bound the cost and survive a dead pod |
| B | **mainnet 962,000**, 4× RTX PRO 6000, aggregate spread | `gpu/gpu-libzync-multi.sh` on a coin-log bundle | wall time against design B's **174 s** on the same box shape (GPU_RESULTS.md) | cycles +2.7% (execute mode), so about 180 s. A DEMO proof: a modelled log around one real block (`cl_synth_around`), cycles not history. ⚠ The script predates the consolidated build (it sets round-4 levers and an ECMH hints file): it needs the pins + `HAZYNC_LIBZYNC` + `HAZYNC_COINLOG` build and `HAZYNC_COINLOG_TXCOUNTS` in the bundle, dry-run on the laptop with `HAZYNC_AGG_EXECUTE=1` first |
| C | **live tip hour**, 4× PRO 6000 | `gpu/fleet/tiprun.sh` | detect-to-verified per block against 191 s (design B, 10-29) | needs a real mainnet log at the tip ⇒ ⚠ not possible until the disk-backed store and a log built from genesis exist; skip for now |

## 2. Checks after each run

- `libzync-verify <final receipt> -` on the laptop: every line `[ok]`, verdict OK (A) or DEMO (B).
- A: `coinlog_ref.py <blocks> 2600 state_2600.json` → `COINLOG_REF_MATCH`;
  `STATES_OUT=… coinlog_ref.py …` then `block_forest_ref.py check hashes_2600.txt states.jsonl state_2600.json` →
  `BLOCK_FOREST_OK`; `genesis_anchor_id.py signet <id> state_2600.json` → `ANCHOR_MATCH`.
- Negative: a flipped byte in the receipt, the wrong network's image, a design-B image's receipt: each FAIL.
- Harvest before release (the run scripts do it; check the files are on the laptop before the pod is gone).

## 3. Not covered here

- Mainnet from genesis with the coin log: needs the disk-backed log store (COINLOG.md §8), then a cost estimate.
- The transaction log (`txproofs/README.md`): a design, not built.
