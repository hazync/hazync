# Utreexo roots near the tip (for Floresta) — NOT DEPLOYED

`hazync-utreexo-follow` keeps the utreexo zyncAPP's proof a few blocks below Bitcoin's tip (docs/libzync/UTREEXO_ZAPP.md
§10). It serves ONE receipt, the newest: Floresta's accumulator (roots, leaf count) after block
H = min(node tip − MARGIN, the core proof's height, last + MAX_STEP). Each update is a step that verifies the previous
receipt inside the zkVM and applies the new blocks with rustreexo's `Stump::modify`, so the newest receipt alone proves
everything from the base (a walk from genesis, `zyncapp utreexo-prove`, installed once with `--set-base`).

Nothing is published unless `zapp-utreexo verify`, given the node's own hashes at H and at the core tip, says OK (or
DEMO with `ALLOW_DEMO=1`: every proof is DEMO while the core proof is below minimum
work). Published folder: `latest.json` (written last), `latest.rcpt` (the stable name), `utreexo-<H>.rcpt` (the last 3).
Proposed URL: `https://hazync.org/utreexo-live/` (an nginx alias outside the site root, as `/headers-live/`).

Reorgs: below MARGIN nothing happens; a changed block at the published H makes the follower fall back to the newest kept
state (`state/`, the last `KEEP`) still on the node's chain, move what is above it to `orphaned/` and step forward; an RPC failure or a node below the proven height is an error, never a reorg; a core tip that is not the node's waits
for the next core proof; a reorg below the base stops (install a new base).

## The witness source

The step guest needs, per block, the spent coins' data and the batch proof of the block's deletions. They come from
`zyncapp utreexo-tip-prove --source forest` (`WITNESS_SOURCE=forest`, the default; UTREEXO_ZAPP.md §10.5.1): Hazync's
own full Utreexo forest (a compact port of rustreexo 0.7.0's MemForest), saved after every step as
`$UTREEXO_DIR/forest-<H>-<hash>.bin`, and the spent coins' data from the block files' prevouts (the node's undo data,
`getblock` verbosity 3 via `fetch_block_rpc.py`). The source holds no UTXO set. Every block is checked natively
(rustreexo `Stump::modify`) before anything is proved, and the guest checks it again: the source is never trusted.
A saved forest is refused at load if its checksum, any inner hash (all are recomputed), its counts or its roots are
wrong. `WITNESS_SOURCE=native` is the old test-scale replay (every live coin and the forest as `Rc` nodes in memory),
kept for comparison.

**Losing a forest** (review F1). A step starts from the newest kept forest AT OR BELOW its previous height whose block is
the node's, and replays forward from there over the block files (only those are fetched); `zyncapp` checks the result
is the previous receipt's block, leaf count and roots. Forests above the new H (left by a kill during a fall-back or a
`--set-base`) move to `orphaned/` and take no retention slot. `DRY_RUN=1` adopts and prunes nothing. The forests of the `FOREST_KEEP` newest states are kept
(default `KEEP + 1`: one per kept state, so any fall-back finds one at or below it). A run killed between switching
`latest` and renaming its forest leaves it in `work/forest.next`: the next run adopts it when its header names `latest`.
`--set-base` and a fall-back move forests to `orphaned/`, never delete them; forests there go after
`ORPHANED_FOREST_MIN` (2 days). The follower passes the chosen forest with `--cache-required`: one `zyncapp` cannot use
(corrupt, the other source's format after a `WITNESS_SOURCE` switch, not at or below P) is exit 4, and the follower
moves it to `orphaned/unusable-*` and tries the next older one, instead of `zyncapp` silently replaying from genesis
(whose bodies a pruned follower does not have). Only with NO usable kept forest at or below the previous height does the source replay from
genesis, and above `GENESIS_REPLAY_MAX` (100,000) blocks that is refused unless `ALLOW_GENESIS_REPLAY=1` (at the tip
it fetches ~970,000 bodies with prevouts onto this disk).

**Memory** (review F3). `zyncapp` saves the source at H before proving (`<cache-out>.pending`, renamed after the
statement is checked) and drops it, so the forest is not held through the proof. After each step the follower writes
the forest's heap to `$UTREEXO_DIR/forest-heap`, logs a warning above `FOREST_HEAP_WARN_BYTES` (12 GiB, half of the
unit's `MemoryMax=24G`), and `hazync-check-utreexo-follow` alerts on it.

Sizes (UTREEXO_ZAPP.md §10.5.1): MEASURED on a synthetic forest of 8M live leaves: ~130 bytes of heap per live leaf after a load
(MemForest: ~3x), the saved file 66 bytes per live leaf. ESTIMATED at the tip's ~165M live coins: ~21 GB of memory and
an ~11 GB file per kept forest.

## Settings (environment)

⚠ `BLOCKS` must be the follower's own directory: it moves block files out of it (reorgs, stale files) and deletes them
(`PRUNE_BLOCKS`). Create it with `hazync-utreexo-follow --init`, which writes the marker `.hazync-utreexo-blocks`
naming `UTREEXO_DIR`; the follower refuses to run on a directory without it, and `--init` refuses a non-empty unmarked
directory.

⚠ `PUBLISH` must be a DEDICATED target: the upload runs `rsync --delete-after`, which deletes everything else there.

`UTREEXO_DIR`, `UTREEXO_BIN`, `ZYNCAPP`, `VERIFY` (zapp-utreexo), `ELF` / `WANT_ELF` (the tip guest and its sha256),
`BASE_ID` / `TIP_IMAGE` (image ids, default the built-in ones), `CORE` (the core receipt, default the spine copy),
`BLOCKS` and `FETCH` (block files and how to fetch them), `BITCOIN_CLI`, `MARGIN` (6), `MAX_STEP` (2016), `KEEP` (12),
`WITNESS_SOURCE` (forest), `FOREST_KEEP` (`KEEP` + 1, at least 1), `GENESIS_REPLAY_MAX` (100000) /
`ALLOW_GENESIS_REPLAY` (0), `FOREST_HEAP_WARN_BYTES` (12 GiB), `ORPHANED_FOREST_MIN` (2880), `PRUNE_BLOCKS` (0; the unit
sets 1), `ALLOW_DEMO` (0),
`NETWORK` (mainnet), `PUBLISH` / `PUBLISH_KEY` (rsync target). `fetch_block_rpc.py` reads `HAZYNC_RPC_COOKIE`.

## Install (hazync-proof) — the procedure, NOT EXECUTED

Where: hazync-proof (bitcoind with txindex at 127.0.0.1:8332, the spine at `/var/lib/hazync-lz/spine/spine.bin`).
State on the root disk (`/var/lib/hazync-utreexo`; not `/srv/bulk`, whose writes are capped at ~40 MB/s and a forest
file is rewritten every step). Disk: block files for one step (pruned after it), `FOREST_KEEP` forests (13 by default:
~143 GB at the tip, ESTIMATE; tiny at today's heights), orphaned forests for 2 days, `KEEP` receipts of ~230 KB.
Checked read-only on hazync-proof (20.11.2026): `bitcoind.service`, `hazync-alert@.service`,
`/usr/local/sbin/hazync-run-check`, `/usr/local/bin/bitcoin-cli`, the cookie `/var/lib/bitcoind/.cookie`, python3,
flock and rsync are there; the root disk had 1.7 T free.

⚠ **Order.** Install only from a commit that is ON `main`: the box's `hazync-check-unit-drift` compares every installed
`hazync-*` unit with a fresh clone of `origin/main`, so a unit from an unmerged branch is reported as drift.
Every setting lives in the repo's unit files: publishing (`PUBLISH`) and a larger `MemoryMax` are changes to
`hazync-utreexo-follow.service` here, merged, then installed — never a drop-in made on the box (it is drift too, and a
repo drop-in that is not installed is reported as missing).

On a build box, from the commit being deployed:

```sh
(cd prover && RISC0_SKIP_BUILD=1 cargo build --release -p zyncapp-host)            # zyncapp; the ELF comes from the container
cargo build --release --manifest-path zapps/utreexo/cli/Cargo.toml --bin zapp-utreexo
docker build --secret id=gh_token,env=GITHUB_TOKEN -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .   # as CI does
mkdir -p elf && docker run --rm -v "$PWD/elf:/out" hazync-libzync-repro cp /root/libzync-elf/zyncapp-utreexo-tip.bin /out/
sha256sum elf/zyncapp-utreexo-tip.bin    # must be 9c5c04c61e6e0bf5fe29e6f727d1c9a61af411d8070224a09f7c08a0fe5c372c (LIBZYNC_IDS)
```

On hazync-proof, as root (copy the files above, `prover/fetch_block_rpc.py`, the two unit files here and a base receipt:
`zapps/utreexo/vectors/mainnet-3.receipt` is a real base proof of H = 3 under the pinned base image `00b2d1e8`):

```sh
install -d /opt/hazync-utreexo/bin /var/lib/hazync-utreexo
install -m755 hazync-utreexo-follow zyncapp zapp-utreexo fetch_block_rpc.py /opt/hazync-utreexo/bin/
install -m644 zyncapp-utreexo-tip.bin /opt/hazync-utreexo/bin/
cp hazync-utreexo-follow.service hazync-utreexo-follow.timer /etc/systemd/system/ && systemctl daemon-reload
# the unit's environment, for the manual steps below
E="UTREEXO_DIR=/var/lib/hazync-utreexo UTREEXO_BIN=/opt/hazync-utreexo/bin ELF=/opt/hazync-utreexo/bin/zyncapp-utreexo-tip.bin
   WANT_ELF=9c5c04c61e6e0bf5fe29e6f727d1c9a61af411d8070224a09f7c08a0fe5c372c CORE=/var/lib/hazync-lz/spine/spine.bin
   HAZYNC_RPC_COOKIE=/var/lib/bitcoind/.cookie ALLOW_DEMO=1"
export PATH=/usr/local/bin:$PATH    # bitcoin-cli is in /usr/local/bin, not on a non-interactive ssh shell's PATH
env $E /opt/hazync-utreexo/bin/hazync-utreexo-follow --init
env $E /opt/hazync-utreexo/bin/hazync-utreexo-follow --set-base mainnet-3.receipt 3 0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449
env $E DRY_RUN=1 /opt/hazync-utreexo/bin/hazync-utreexo-follow     # logs the H it would prove; proves nothing
systemctl start hazync-utreexo-follow.service && journalctl -u hazync-utreexo-follow -n 30   # one real step, by hand
systemctl enable --now hazync-utreexo-follow.timer
# the check (stall and forest memory), through the box's hazync-run-check: enable it only AFTER the first manual step
# above has published (until then there is nothing to measure and it would page "nothing published")
install -m755 hazync-check-utreexo-follow.sh /usr/local/sbin/hazync-check-utreexo-follow
cp hazync-check-utreexo-follow.service hazync-check-utreexo-follow.timer /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now hazync-check-utreexo-follow.timer
```

The first steps replay and prove from the base (H = 3) to the spine's height in runs of `MAX_STEP` blocks; the forest
is then saved and later runs start from it. Nothing is uploaded until `PUBLISH` is set (in the unit, by a repo change:
`Environment=PUBLISH=root@<web box>:./`), after the web box has the target: a key pinned with
`command="/usr/bin/rrsync -wo /var/www/hazync-utreexo",restrict` and an nginx alias `/utreexo-live/` outside the site
root, as for `/headers-live/` (coordinator/deploy/headers/README.md). Before the core proof nears the tip, raise
`MemoryMax` in the unit (the forest's ESTIMATED ~21 GB at the tip while it is loaded and saved; the prover's own peak
after it is dropped) and `FOREST_HEAP_WARN_BYTES` in both units, and check the root disk for the forest files.

**The timer and the load** (review F7). At the tip a run loads the forest (~4 min, ESTIMATE) and saves it (~40 s) besides
proving. The timer fires every 10 minutes, but a oneshot that is still running is not started again (and the
follower's `flock` would turn a second run away), so a long run only delays the next one. A long-running source that
keeps the forest in memory would remove the load; it is not built.

Undo: `systemctl disable --now hazync-utreexo-follow.timer`, remove the two units, `/opt/hazync-utreexo` and
`/var/lib/hazync-utreexo`. Nothing else on the box is touched.

## Test

`test/run_ci.sh` (CI, job `utreexo-follower`; ~8 min on a laptop): `test_check.sh` (the check on every path) and its
control, then `test_follow_logic.sh` on header-only block files made from `test/headers_1_500.txt` (mainnet 1..500,
`mkblocks.py`), and six controls run beside it, each the follower with one guard taken out (the old fall-back, the old
retention, no adoption of `work/forest.next`, no forest below P, no genesis-replay limit, `--set-base` deleting
forests): each must fail checks.

`test/test_follow_logic.sh <block dir with block_1..300.json>`: the script's own logic with a stub prover and checker
(seconds): RPC failures and a node below the proven height are errors, reorg fall-back and what moves aside, retention,
the republish under a newer core proof, the state symlinks, headers-only links, block pruning, the forest source asked
for, the `FOREST_KEEP` newest forests kept, a fall-back starting from the kept forest at its height. Controls: the
script with the old fall-back (every forest moved aside) and with the old retention (only the newest kept) each fail
2 checks.

`test/test_follow.sh <zyncapp> <zapp-utreexo> <base receipt, mainnet H = 3> <core receipt 1..T> <block dir>` runs the
script against a fake bitcoin-cli over real mainnet block files, with the real prover and checker (three CPU steps):
base install (a wrong hash refused), first step published, an unchanged tip, a step from a step, a step that does not
verify and a wrong ELF (the folder untouched), a reorg below the published height (fall back, step forward), a reorg
below the base (stop).

The witness source itself: `zapps/utreexo/tip-step` (`forest_tests.rs`, CI's zapps job: the forest against MemForest,
lying nodes and stores, restarts) and `zapps/utreexo/tip-source-tests.sh` (executed on mainnet blocks 1..100,000: the
same witness bytes as the native replay, restarts, lies refused by the source and by the guest).
