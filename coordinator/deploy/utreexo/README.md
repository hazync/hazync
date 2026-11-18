# Utreexo roots near the tip (for Floresta) — NOT DEPLOYED

`hazync-utreexo-follow` keeps the utreexo zyncAPP's proof a few blocks below Bitcoin's tip (docs/libzync/UTREEXO_ZAPP.md
§10). It serves ONE receipt, the newest: Floresta's accumulator (roots, leaf count) after block
H = min(node tip − MARGIN, the core proof's height, last + MAX_STEP). Each update is a step that verifies the previous
receipt inside the zkVM and applies the new blocks with rustreexo's `Stump::modify`, so the newest receipt alone proves
everything from the base (a walk from genesis, `zyncapp utreexo-prove`, installed once with `--set-base`).

Nothing is published unless `zapp-utreexo verify`, given the node's own hashes at H and at the core tip, says OK (or
DEMO with `ALLOW_DEMO=1`: every proof is DEMO while the images are local builds and the core proof is below minimum
work). Published folder: `latest.json` (written last), `latest.rcpt` (the stable name), `utreexo-<H>.rcpt` (the last 3).
Proposed URL: `https://hazync.org/utreexo-live/` (an nginx alias outside the site root, as `/headers-live/`).

Reorgs: below MARGIN nothing happens; a changed block at the published H makes the follower fall back to the newest kept
receipt (`hist/`, the last `KEEP`) still on the node's chain and step forward; a core tip that is not the node's waits
for the next core proof; a reorg below the base stops (install a new base).

⚠ The deletion proofs come from a native MemForest replay (cached in `forest.bin`): test scale only. At the tip the
witness needs a Utreexo bridge (utreexod's proof index) or a disk forest; neither is built (§10.5).

## Settings (environment)

`UTREEXO_DIR`, `UTREEXO_BIN`, `ZYNCAPP`, `VERIFY` (zapp-utreexo), `ELF` / `WANT_ELF` (the tip guest and its sha256),
`BASE_ID` / `TIP_IMAGE` (image ids, default the built-in ones), `CORE` (the core receipt, default the spine copy),
`BLOCKS` and `FETCH` (block files and how to fetch them), `BITCOIN_CLI`, `MARGIN` (6), `MAX_STEP` (2016), `KEEP` (12),
`ALLOW_DEMO` (0), `NETWORK` (mainnet), `PUBLISH` / `PUBLISH_KEY` (rsync target).

## Test

`test/test_follow.sh <zyncapp> <zapp-utreexo> <base receipt, mainnet H = 3> <core receipt 1..T> <block dir>` runs the
script against a fake bitcoin-cli over real mainnet block files, with the real prover and checker (three CPU steps):
base install (a wrong hash refused), first step published, an unchanged tip, a step from a step, a step that does not
verify and a wrong ELF (the folder untouched), a reorg below the published height (fall back, step forward), a reorg
below the base (stop).
