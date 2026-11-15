# Board reset to 46666b6f (v0.26.0)

> ⏰ **NOT DONE YET.** This is the plan for the box `hazync-proof`, written on 2026-11-15 from the repo at `1b4ac3db`
> and a read-only survey of the box. Nothing here runs by itself: each step is done by hand, in order, and each ends
> with a check. A step whose check fails stops the reset; its rollback is under it.

Release v0.26.0 commits each block's undo data in the block forest ([`docs/libzync/UNDO.md`](../../../docs/libzync/UNDO.md)).
Its mainnet image `46666b6f…` has an **empty lineage**, so the public board **restarts from genesis** and every block
is proved again. The handover machinery (`LZ_HANDOVER_AT`, [`docs/HANDOVER.md`](../../../docs/HANDOVER.md)) stays in the code
for later upgrades and is switched **off** here. The paper prep run (`libzync-prep`) is **not** restarted: it is
re-forested and continues (UNDO.md §8).

The previous reset, v1 to v2 under `8888d59c` on 2026-11-12, is [`SWITCH.md`](SWITCH.md) (step 0 and section 4c).
This page follows it wherever nothing has changed and says so.

## What the box held at the survey (2026-11-15, read-only)

| | |
|---|---|
| board | `8888d59c`, frontier 1,999, 3,206 receipts, spine 1..1,999; `/api/meta` handover era `pre` at 2,000, so it has **not moved for about a day** (`blocked: 2000`) |
| board pass | `libzync-board-prep` at 5,998, 5,000 leaf bundles (the window) |
| paper prep | `libzync-prep` at save point 607,000; 1,000 blocks in 3,842 s there (0.26 blocks/s, of which 403 s is the save); `out` = `/var/lib/libzync-fast/out`, 609,064 files, 2.4 GB; host `863c9ce3` |
| zyncAPP | follower at 1,999 (`/srv/bulk/libzync-zapp`, host `8888d59c`) |
| undo cache | `hazync-coord:/root/undocommit/precompute/reforest/undo.tsv`: heights 1..620,000, 94 MB, sha256 `81f2532f6ee1557c6879e7bf4d2d3bcd54b042e83722b7453fca18217f6891fd` |

**Found only on the box** (not in the repo, or different from it):

- `hazync-lz-coordinator.service.d/zz-handover.conf` (`LZ_HANDOVER_AT=2000`). #730 removed it from the repo. With the
  v0.26.0 host (no handover in its lineage) the coordinator would stay in the `pre` era and **cap the new board at
  1,999 for good**, every unit healthy (`server.py` `handover_era`). Step 5 removes it; `check-deployment.sh --local`
  now fails a board capped that way.
- `/usr/local/sbin/hazync-lz-zapp-follow` predates #730: it cannot count `history.bin` records that carry an undo hash
  (bit 31). `/usr/local/sbin/hazync-lz-prune-leaves` and the installed `hazync-lz-prune-leaves.service` still default
  to port 8900 (the box's `zz-libzync.conf` drop-in covers it). Step 6 reinstalls all three.
- `/opt/hazync-lz/bin/zync-serve` is from 2026-11-09; it must be rebuilt from the release commit to read the new
  `history.bin` (it is not a release asset, `scripts/release.sh` step 3).
- `hazync-lz-offsite-spine{,-b2}.timer` are **disabled**: they failed on 11-12 on the empty spine and were never
  enabled afterwards (SWITCH.md step 5), so **the `8888d59c` spine was never copied off the box**. Step 1 copies it once.
- `hazync-lz-coordinator`, `libzync-board-prep`, `libzync-board-fetch`, `libzync-prep` and `libzync-fetch` are
  `disabled`: after a reboot the public board does not come back by itself. Not changed here (question 5 below).
- The web box's `hazync-check-spine-remote` still runs the CORE verifier (guest `37987b85`) and has failed every run
  since the 11-12 switch (444 in a row on 2026-11-15). Step 13 retargets it (the repo unit is fixed in this PR).
- `check-unit-drift` on the box reports one drift today: `zz-handover.conf` / `LZ_HANDOVER_AT`. It goes with step 5.
  The unit changes in this PR change values only (`HAZYNC_METHOD_ID`, `--method-id`), which the drift check reports
  as a note, not drift, so installing them shows no false drift.

## Decisions (recommended; the owner confirms)

| | 11-12 precedent (SWITCH.md) | here |
|---|---|---|
| coordinator data | `VACUUM INTO /root/lz-863c9ce3.db`, `mv /var/lib/hazync-lz /var/lib/hazync-lz.863c9ce3`, a fresh directory with its subdirectories | the same, as `.8888d59c`. The old directory stays read-only as the record |
| contributors | carried (`carry-contributors.py`): handle, key, first seen, last version; not block counts | the same, from the `8888d59c` ledger |
| sponsorships, payments | stayed in the old ledger | the same. Donations come from BTCPay, not the ledger, so `/api/donations` is unaffected |
| offsite copies | new prefixes from the new id; the old ones kept, nothing deleted | `proofs-46666b6f/`, `spine-46666b6f/` (the units' `--method-id`); `proofs-8888d59c/`, `spine-8888d59c/` stay as the record |
| Litestream | v2 ledger added as `coordinator-lz-server1` | the fresh ledger under a NEW replica path `coordinator-lz-46666b6f-server1`, so it never meets the `8888d59c` replica, which stays |
| board pass | emptied (`rm`) and restarted from genesis | restarted from genesis, but the old directories are **moved aside** (same filesystem, instant) so the step can be rolled back. Re-foresting it instead is not possible: its leaves 2..998 are already pruned |
| paper prep | kept running | re-forested and continued (UNDO.md §8), not restarted |
| zyncAPP facts | the 1..200 demo, then the follower at the spine | a FRESH `ZAPP_ROOT`; facts only from a `46666b6f` proof (step 9) |
| `GENESIS_TIP` | `852c5c7e…` | unchanged: `libzync-genesis-tip` of the v0.26.0 mainnet host (hazync-coord build, 2026-11-15) is `852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167` |
| check-board-progress | n/a | nothing to do: a lower proven count is taken as a new board and a new baseline (#719) |

## Names used below

```bash
O=8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4      # the board's image now
N=46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c      # v0.26.0
R=/root/v026                                                              # the release's assets on the box
CLI="bitcoin-cli -datadir=/var/lib/hazync/bitcoin-client"                 # the box's node; the bridge units use this datadir
P=/srv/bulk/libzync-prep                                                  # the paper run's LZ_ROOT (out -> /var/lib/libzync-fast/out)
B=/srv/bulk/libzync-board                                                 # the board pass's LZ_ROOT
```

All commands run as root on `hazync-proof` unless a step names another machine. Never print a drop-in or config that
holds keys (`btcpay`, `sponsor-*`, `zz-proxies`, `zz-signed-claims`, `/etc/litestream.yml`, `/etc/hazync/backup/*`):
key names only.

## 0. Before anything stops

1. **This PR is in the release commit.** The box installs its units and scripts from `/opt/hazync-lz` (step 6), so
   the commit that checkout moves to must carry the `46666b6f` units. If v0.26.0 is tagged before this PR merges,
   deploy `/opt/hazync-lz` at the merge commit instead of the tag. Check, on the laptop:
   `git show v0.26.0:coordinator/deploy/libzync/hazync-lz-check-spine.service | grep -c "$N"` prints 1.
2. **Stop the sponsor bot before the release is published.** It takes the `latest` release and boots pods whose host
   must match the board's `method_id`; between the release and step 7 they would not.
   `systemctl stop hazync-sponsor-bot.timer`; check `systemctl is-active hazync-sponsor-bot.timer` = `inactive`.
   Rollback: `systemctl start hazync-sponsor-bot.timer`.
3. **Release v0.26.0 is published** with the 13 assets of `scripts/release-assets.txt` and signed. Check on the laptop:
   `gh release view v0.26.0 -R hazync/hazync --json assets -q '.assets|length'` = 13, and the `SHA256SUMS.txt.asc`
   signature verifies. ⛔ `release.sh --from tag` can publish an EMPTY release and call it verified: count the assets.
4. **The release's assets on the box**, checked against the signed sums:
   ```bash
   mkdir -p $R && cd $R
   for f in hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu hazync-verify.wasm SHA256SUMS.txt SHA256SUMS.txt.asc; do
     curl -sfLO https://github.com/hazync/hazync/releases/download/v0.26.0/$f; done
   grep -E ' (hazync-host-x86_64-linux-gnu|hazync-verify-x86_64-linux-gnu|hazync-verify.wasm)$' SHA256SUMS.txt | sha256sum -c -
   chmod 755 hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu
   ./hazync-host-x86_64-linux-gnu method-id | head -1          # METHOD_ID 46666b6f…
   ./hazync-host-x86_64-linux-gnu libzync-lineage | head -2    # HANDOVER none
   ./hazync-host-x86_64-linux-gnu libzync-genesis-tip | tail -1  # 852c5c7e…0167 (the unit's GENESIS_TIP)
   sha256sum hazync-verify.wasm                                  # the website pins this (step 12)
   ```
   Check: all three `OK`, the id, `HANDOVER none`, the genesis tip. The wasm sha is compared with
   `swiftsync-verify-wasm/README.md`'s `12bbc9ae…` (#730). ⚠ #732 reports a local `build.sh` wasm of `486ae6ed…`; if
   the release's differs from `12bbc9ae…`, stop and settle which one the site pins (question 3).
5. **zync-serve from the release commit**, built where cargo is (hazync-coord), copied through the laptop:
   ```bash
   # on hazync-coord, in a checkout at v0.26.0
   cargo build --release --locked --manifest-path swiftsync-verify/Cargo.toml --bin zync-serve
   # on the laptop
   scp -3 hazync-coord:<checkout>/swiftsync-verify/target/release/zync-serve hazync-proof:$R/zync-serve
   ```
   Check: `sha256sum` equal at both ends.
6. **The undo cache, staged** (not yet in the run's directory):
   ```bash
   # on the laptop (the boxes do not trust each other)
   scp -3 hazync-coord:/root/undocommit/precompute/reforest/undo.tsv hazync-proof:/root/undo-cache-620000.tsv
   ```
   Check on hazync-proof: `sha256sum /root/undo-cache-620000.tsv` = `81f2532f…91fd`, `wc -l` = 620,000,
   `tail -n1 | cut -f1` = 620000. ESTIMATE: a few minutes for 94 MB through the laptop (not measured for this file).
7. **Rehearse the re-forest on a copy (recommended), while prep runs.** It is the one step that rewrites the paper
   run in place, and the run's 607,000 states were written by several hosts (`863c9ce3` now): the walk refuses a state
   whose forest is not the pre-commitment one, and a stop after it has started leaves states of both formats. The
   copy shows that the whole walk passes and measures how long it takes on this box (phase 2 is measured only for 2,000
   states: under a second on hazync-coord, UNDO.md §8). It reads the run's files and writes only the copy, on the same
   disk (1.5 TB free):
   ```bash
   S0=$(cd $P/out && for h in $(ls state_*.smt | sed 's/state_\([0-9]*\)\.smt/\1/' | sort -rn); do
          [ -s state_$h.json ] && [ -e state_$h.coinlog ] && [ -s state_$h.trie ] && { echo $h; break; }; done)
   W=/var/lib/libzync-fast/rehearse-reforest; mkdir -p $W/out/reforest $W/leaves
   ionice -c3 rsync -a --include="state_*.json" --exclude="*" $P/out/ $W/out/
   cp $P/out/state_$S0.coinlog $W/out/ && cp $P/leaves/leaf_*.bin $W/leaves/
   cp /root/undo-cache-620000.tsv $W/out/reforest/undo.tsv
   ls $W/out | sed -n 's/^state_\([0-9]*\)\.json$/\1/p' | awk -v s=$S0 '$1 > s {print "state_" $1 ".json"}' | (cd $W/out && xargs -r rm)
   time env -u HAZYNC_COINLOG_DIR -u HAZYNC_TRIE_DIR -u HAZYNC_PREP_LEAF_BUNDLES nice -n 10 ionice -c3 \
     $R/hazync-host-x86_64-linux-gnu libzync-reforest $W/out $S0 --bitcoin-cli "$CLI" --all --bundles $W/leaves 2>&1 | tail -8
   ```
   Check: it ends with `LZ_REFOREST_DONE 1..=$S0`, `$W/out/state_$S0.forest` holds `U0`, and the cross-check line says
   the leaf bundles at or below $S0 (170 .. 550,000) agree with the node. Note the time: it is the estimate for step 3.
   Then `rm -rf $W`. A failure here costs nothing: the run is untouched. Read the stop message and bring it to the
   owner before going on.

## 1. Stop the writers, and copy the old board's record off the box

1. **The paper prep, at a save point.** It saves every 1,000 blocks; a save is complete when its `PREP_PROGRESS` line
   is printed (the `.trie` marker is written last). Stop it right after one, so nothing is redone:
   ```bash
   journalctl -u libzync-prep -f -n0 -o cat | grep -m1 PREP_PROGRESS; systemctl stop libzync-prep
   ```
   ESTIMATE: the wait is at most one save interval, about 64 min at 607,000 (3,842 s per 1,000 blocks, measured).
   `libzync-fetch` (its block source) keeps running: it writes only `$P/blocks`, which re-forest never opens.
   Check: `systemctl is-active libzync-prep` = `inactive`; S, the newest complete save point:
   ```bash
   S=$(cd $P/out && for h in $(ls state_*.smt | sed 's/state_\([0-9]*\)\.smt/\1/' | sort -rn); do
         [ -s state_$h.json ] && [ -e state_$h.coinlog ] && [ -s state_$h.trie ] && { echo $h; break; }; done); echo S=$S
   ```
   S is a multiple of 1,000 and equals the height of the last `PREP_PROGRESS` line. Rollback: `systemctl start libzync-prep`.
2. **The board's writers and the timers that would act on a half-reset board:**
   ```bash
   systemctl stop libzync-board-prep libzync-board-fetch
   systemctl stop hazync-lz-prune-leaves.timer hazync-lz-zapp-follow.timer libzync-xcheck.timer \
     hazync-lz-offsite-proofs.timer hazync-lz-offsite-proofs-b2.timer \
     hazync-lz-check-spine.timer hazync-lz-check-proofs.timer hazync-check-board-progress.timer \
     hazync-check-bundle-gap.timer hazync-check-continuity.timer hazync-retention-check.timer \
     hazync-offsite-watch.timer hazync-offsite-summary.timer hazync-check-unit-drift.timer \
     hazync-coordinator-backup.timer hazync-offsite-ledger-b2.timer hazync-offsite-keys.timer hazync-offsite-keys-b2.timer
   ```
   ⛔ The offsite timers must stay stopped until step 10: from step 6 the units name `46666b6f`, the copy is
   append-only by name, and an `8888d59c` receipt uploaded under `proofs-46666b6f/` would stay there for good.
   Check: `systemctl list-timers --all 'hazync-*' 'libzync-*'` shows each as inactive (n/a), and
   `systemctl list-units --state=activating,active 'hazync-lz-prune*' 'hazync-lz-zapp*' 'libzync-xcheck*' 'hazync-lz-offsite*'`
   is empty (a run in progress finishes first). The coordinator and zync-serve stay up (the board serves its 1,999).
   Rollback: `systemctl start` the same list.
3. **The old board's last offsite copies**, with the units as they are (they name `8888d59c`):
   ```bash
   systemctl start hazync-lz-offsite-proofs.service hazync-lz-offsite-proofs-b2.service
   systemctl start hazync-lz-offsite-spine.service hazync-lz-offsite-spine-b2.service   # never ran on a real spine
   for u in hazync-lz-offsite-proofs hazync-lz-offsite-proofs-b2 hazync-lz-offsite-spine hazync-lz-offsite-spine-b2; do
     echo "$u $(systemctl show -p Result --value $u)"; journalctl -u $u -n 3 -o cat --no-pager; done
   ```
   Check: each `Result=success`; the spine copies name `spine-8888d59c/… [1..1,999]` (R2 and B2). A failure here is
   not a reason to go on without the record: fix it first. Rollback: none needed (append-only copies).

## 2. The undo cache into the run

```bash
mkdir -p $P/out/reforest && cp /root/undo-cache-620000.tsv $P/out/reforest/undo.tsv
```
The cache runs past S (to 620,000); re-forest uses heights 1..S of it and leaves the file as it is. Check:
`cmp /root/undo-cache-620000.tsv $P/out/reforest/undo.tsv`. Rollback: `rm -r $P/out/reforest`.

## 3. Re-forest the paper run (UNDO.md §8 step 3)

1. **Back up what re-forest rewrites** (every `state_<h>.json` 1..S and the leaf bundles; the stores are never opened):
   ```bash
   tar -C /var/lib/libzync-fast -cf /var/lib/libzync-fast/out-pre-reforest-$S.tar out
   tar -C $P -cf /var/lib/libzync-fast/leaves-pre-reforest-$S.tar leaves
   ```
   Check: `tar -tf …out….tar | grep -c 'state_.*\.json$'` equals `ls $P/out | grep -c '^state_.*\.json$'`.
2. **Run it** as a transient unit, so a dropped ssh session does not kill it:
   ```bash
   systemd-run --unit=libzync-reforest --property=Nice=10 --property=IOSchedulingClass=idle \
     --setenv=HOME=/root $R/hazync-host-x86_64-linux-gnu libzync-reforest $P/out $S \
     --bitcoin-cli "$CLI" --all --jobs 4 --bundles $P/leaves
   journalctl -u libzync-reforest -f -o cat
   ```
   (`systemd-run` starts a clean environment: `HAZYNC_COINLOG_DIR`, `HAZYNC_TRIE_DIR` and `HAZYNC_PREP_LEAF_BUNDLES`
   are unset, as re-forest requires. `--jobs 4` = the node's default `rpcthreads`; the box's bitcoin.conf sets none.)
   - Phase 1 finds the cache, fetches 32 heights again and must print `resuming after 620000 cached heights; … all
     agree`. Nothing past S is fetched.
   - Phase 1b: `N leaf bundle(s) cross-checked` with N = the `HAZYNC_PREP_LEAF_AT` heights at or below S (11 at
     S = 608,000: 170, 1,000, 100,000 … 550,000).
   - Phase 2 walks 1..S. ESTIMATE: the rehearsal's time (step 0.7); unmeasured otherwise.
   Check: `LZ_REFOREST_DONE 1..=S`, `cat $P/out/state_$S.forest` = `U0`, `$P/out/reforest/DONE_$S` exists, and
   `systemctl show -p Result --value libzync-reforest` = `success`. A run that stopped can be run again with the same
   command (it accepts states of both formats and resumes the cache), once the cause is understood.
   Rollback (before step 4 only): `rm -r $P/out/reforest`, restore both tars over `out` and `leaves`, then
   `systemctl start libzync-prep` with the old host still in place.

## 4. The new host for the paper prep, and continue

```bash
cp -a $P/bin/host_lz $P/bin/host_lz.863c9ce3
install -m 755 $R/hazync-host-x86_64-linux-gnu $P/bin/host_lz
$P/bin/host_lz method-id | head -1                       # 46666b6f…
systemctl start libzync-prep
journalctl -u libzync-prep -n 20 -o cat --no-pager        # "libzync-prep: S+1..974936 from state_S.json", re-stamped
```
Check: the runner names `state_$S.json`, prep prints the re-stamp line (image `46666b6f…`) and no refusal about a
forest marker; the next `PREP_PROGRESS` line (one save interval later) gives the new host's rate, to compare with
0.26 blocks/s. The `state_<h>.json` files above S left by the old host are rewritten as prep reaches them.
`HAZYNC_PREP_CHECK` (`$P/ref`, states 1..200) does not apply above S.
⛔ **Point of no return for the paper run:** once the new prep has saved past S, its stores have moved on and the old
format cannot be resumed. Before that save: `systemctl stop libzync-prep`, put `host_lz.863c9ce3` back, and roll back
step 3.

## 5. Switch the handover off

```bash
mv /etc/systemd/system/hazync-lz-coordinator.service.d/zz-handover.conf /root/zz-handover.conf.8888d59c
systemctl daemon-reload
```
It takes effect at the coordinator's next start (step 7). Check:
`systemctl show hazync-lz-coordinator -p Environment --value | tr ' ' '\n' | grep -c LZ_HANDOVER_AT` = 0.
Rollback: move it back, `daemon-reload`.

## 6. `/opt/hazync-lz` at the release commit, and everything installed from it

The coordinator is still up on the old board here. `server.py` is unchanged from `e5ad283` to `1b4ac3db`, so the
deploy script does not restart it (it checks the unit is active and `/api/state` answers):
```bash
DRY_RUN=1 UNIT=hazync-lz-coordinator REPO=/opt/hazync-lz /opt/hazync-lz/coordinator/deploy/deploy-coordinator.sh v0.26.0
UNIT=hazync-lz-coordinator REPO=/opt/hazync-lz /opt/hazync-lz/coordinator/deploy/deploy-coordinator.sh v0.26.0
cd /opt/hazync-lz/coordinator/deploy/libzync
install -m 644 *.service *.timer /etc/systemd/system/
install -m 755 hazync-lz-zapp-follow.sh    /usr/local/sbin/hazync-lz-zapp-follow
install -m 755 hazync-lz-prune-leaves.sh   /usr/local/sbin/hazync-lz-prune-leaves
install -m 755 hazync-lz-verify-spine.sh   /usr/local/sbin/hazync-lz-verify-spine
install -m 755 libzync-xcheck.py           /usr/local/sbin/libzync-xcheck
install -m 755 libzync-prep-run.sh $P/bin/libzync-prep-run.sh && install -m 755 libzync-prep-run.sh $B/bin/libzync-prep-run.sh
install -m 644 /opt/hazync-lz/coordinator/deploy/watchdog/hazync-check-spine-remote.service /root/   # for the web box, step 13
systemctl daemon-reload
```
(`*.service` includes the paper run's own `libzync-prep.service` / `libzync-fetch.service`, unchanged; their
settings are in drop-ins, which stay.) Check: `git -C /opt/hazync-lz describe --tags` = `v0.26.0`;
`grep -l "$O" /etc/systemd/system/hazync-lz-* /etc/systemd/system/libzync-*` prints nothing (SWITCH.md 4c, d);
`grep -c "$N" /etc/systemd/system/hazync-lz-check-spine.service` = 1;
`sha256sum` of each installed script equals the checkout's. Rollback: the same deploy command with `e5ad283`
and the same installs from it.

## 7. Reset the board

1. **Stop it, and Litestream:**
   `systemctl stop hazync-lz-coordinator hazync-lz-zync-serve litestream`. Check all three `inactive`.
2. **The record:**
   ```bash
   sqlite3 /var/lib/hazync-lz/coordinator.db "VACUUM INTO '/root/lz-8888d59c.db'"
   sqlite3 /root/lz-8888d59c.db 'PRAGMA integrity_check; SELECT count(*) FROM contributors; SELECT count(*) FROM ranges'
   mv /var/lib/hazync-lz /var/lib/hazync-lz.8888d59c
   ```
   Check: `ok`; write the counts into the table at the end of this page.
3. **A fresh directory, with every subdirectory** (SWITCH.md 4c, a), and the zyncAPP directory **empty**: the old
   board's facts are about an `8888d59c` proof and are not copied back (step 9 seeds it):
   ```bash
   install -d -o hazync -g hazync /var/lib/hazync-lz /var/lib/hazync-lz/{proofs,spine,witnesses,coord_state,zapp}
   ```
4. **The new hosts and verifier** (each old one kept beside it):
   ```bash
   cd /opt/hazync-lz/bin   # untracked in the checkout
   cp -a host host.8888d59c && cp -a hazync-verify hazync-verify.8888d59c && cp -a zync-serve zync-serve.pre-v026
   install -m 755 $R/hazync-host-x86_64-linux-gnu host
   install -m 755 $R/hazync-verify-x86_64-linux-gnu hazync-verify
   install -m 755 $R/zync-serve zync-serve
   ./host method-id | head -1                         # 46666b6f…
   ```
5. **Litestream: the fresh ledger under a new replica path.** Edit `/etc/litestream.yml` in place without printing it:
   ```bash
   cp -p /etc/litestream.yml /root/litestream.yml.8888d59c
   sed -i 's#^\(\s*path:\s*\)coordinator-lz-server1\s*$#\1coordinator-lz-46666b6f-server1#' /etc/litestream.yml
   grep -cE '^\s*path:\s*coordinator-lz-46666b6f-server1\s*$' /etc/litestream.yml   # 1
   grep -cE '^\s*path:\s*coordinator-server1\s*$' /etc/litestream.yml               # 1 (v1, unchanged)
   ```
   Litestream is started in step 7.7, after the coordinator has created the database.
6. **Start the coordinator once** (it creates the fresh database), **then carry the contributors** with it stopped:
   ```bash
   systemctl start hazync-lz-coordinator && sleep 10 && curl -s 127.0.0.1:8899/api/meta; echo
   systemctl stop hazync-lz-coordinator
   python3 /opt/hazync-lz/coordinator/deploy/libzync/carry-contributors.py --from /root/lz-8888d59c.db --to /var/lib/hazync-lz/coordinator.db
   python3 /opt/hazync-lz/coordinator/deploy/libzync/carry-contributors.py --from /root/lz-8888d59c.db --to /var/lib/hazync-lz/coordinator.db --apply
   chown hazync:hazync /var/lib/hazync-lz/coordinator.db*
   find /var/lib/hazync-lz -not -user hazync             # prints nothing
   systemctl start hazync-lz-coordinator
   ```
   The board pass is still stopped (step 8 restarts it): until it writes leaves there is nothing to claim.
   Check: `/api/meta` = `{"method_id": "46666b6f…", "frontier": 0, "libzync": true, "handover": null, …}` ⛔ a
   `handover` that is not `null` means the drop-in is still there (step 5): stop. The carry plan names the
   `8888d59c` board's contributors; `/api/state` counts only those with work on THIS board, so it says 0 until someone
   proves (SWITCH.md 4b).
7. `systemctl start litestream`; check `journalctl -u litestream -n 20 -o cat` names
   `/var/lib/hazync-lz/coordinator.db` with no error.

Rollback (whole step): stop the coordinator and Litestream; `mv /var/lib/hazync-lz /var/lib/hazync-lz.46666b6f-failed`
and `mv /var/lib/hazync-lz.8888d59c /var/lib/hazync-lz`; the `.8888d59c` / `.pre-v026` binaries back; the
Litestream config back; step 5's rollback; start both.

## 8. The board pass from genesis under 46666b6f

1. **Move the `8888d59c` pass aside** (same filesystems; the `out`, `cl`, `trie` symlinks under `$B` keep pointing at
   `/var/lib/libzync-board/*`, so recreate the targets, never the links):
   ```bash
   for d in out cl trie; do mv /var/lib/libzync-board/$d /var/lib/libzync-board/$d.8888d59c; install -d /var/lib/libzync-board/$d; done
   mv $B/leaves $B/leaves.8888d59c && install -d $B/leaves
   rm -rf $B/blocks && install -d $B/blocks           # re-fetched; prep consumes them anyway
   rm -f /var/lib/hazync-checks/libzync-xcheck.last   # xcheck starts again at 1
   cp -a $B/bin/host_lz $B/bin/host_lz.8888d59c && install -m 755 $R/hazync-host-x86_64-linux-gnu $B/bin/host_lz
   ```
2. **Start it** (the fetcher picks its start height once, at start, so it restarts too):
   `systemctl start libzync-board-fetch libzync-board-prep`.
3. **Check the stamp** (SWITCH.md step 0.7): block 1 starts from genesis, so look at the first state written:
   ```bash
   python3 -c 'import json;print(json.load(open("/var/lib/libzync-board/out/state_1.json"))["self_id"])'
   # [1869309510, 471612798, 2845418815, 153285052, 1779266528, 3091707695, 3338285247, 473752679]  (= 46666b6f… as 8 u32)
   head -c 8 $B/leaves/leaf_1.bin; echo                                                              # LZLEAF02
   ```
   and `journalctl -u libzync-board-prep -n 5 -o cat` says `1..974936 from genesis`.
Rollback: stop both, move the `.8888d59c` directories and host back.

## 9. zync-serve and the zyncAPP follower, fresh

```bash
mv /srv/bulk/libzync-zapp /srv/bulk/libzync-zapp.8888d59c
install -d /srv/bulk/libzync-zapp/bin && install -m 755 $R/hazync-host-x86_64-linux-gnu /srv/bulk/libzync-zapp/bin/host
```
zync-serve cannot start on an empty `/var/lib/hazync-lz/zapp`, and the follower does nothing until the board has a
spine. Two ways to give it facts, both from a `46666b6f` proof (question 2):
- **wait** for the board's first genesis spine and start the follower's timer (step 10): it preps 1..H and starts
  zync-serve itself. Until then `/api/zync/*` answers 502 and Coin check says the fact service is unreachable.
  On 11-12 the first spine reached block 344 in about 2.5 h with 4 pods (`RESULT_board_run_2026-11-12.txt`).
- **seed now** from #732's real receipt of mainnet block 1 (`experimental/libzync-bench/zapp/mainnet_1_1_46666b6f.rcpt`,
  sha256 `35f527be…e5fd`; in the checkout when #732 is in the release), by running the follower once on it. The
  follower's own test of a proof, `verify-any <it> 1` with the v0.26.0 mainnet host, gives `RANGE-OK lo=1 hi=1 …
  anchored=1` on it (hazync-coord's build, 2026-11-15); the rest of this path is not tested:
  ```bash
  install -d /root/zapp-seed && cp /opt/hazync-lz/experimental/libzync-bench/zapp/mainnet_1_1_46666b6f.rcpt /root/zapp-seed/spine.bin
  COORD_SPINE=/root/zapp-seed ZAPP_ROOT=/srv/bulk/libzync-zapp ZAPP_SERVE_DIR=/var/lib/hazync-lz/zapp \
    ZAPP_FETCH=$B/bin/fetch_block_rpc.py HAZYNC_RPC_COOKIE=/var/lib/bitcoind/.cookie /usr/local/sbin/hazync-lz-zapp-follow
  ```
  Check: `serving facts at 1`, `systemctl is-active hazync-lz-zync-serve` = active, `curl -s 127.0.0.1:8601/tip` =
  `{"tip":1}`, `curl -s https://api.hazync.org/api/zync/proof | sha256sum` = the receipt's. From then on the
  timer moves it forward with the board's spine (any spine ends at or above block 1).
Rollback: stop zync-serve, `rm -rf /srv/bulk/libzync-zapp && mv /srv/bulk/libzync-zapp.8888d59c /srv/bulk/libzync-zapp`,
put the old set back from `/var/lib/hazync-lz.8888d59c/zapp`.

## 10. The timers again

```bash
systemctl start hazync-lz-prune-leaves.timer hazync-lz-check-spine.timer hazync-lz-check-proofs.timer libzync-xcheck.timer \
  hazync-check-board-progress.timer hazync-check-bundle-gap.timer hazync-check-continuity.timer hazync-retention-check.timer \
  hazync-coordinator-backup.timer hazync-offsite-ledger-b2.timer hazync-offsite-keys.timer hazync-offsite-keys-b2.timer \
  hazync-offsite-watch.timer hazync-offsite-summary.timer hazync-check-unit-drift.timer
systemctl start hazync-lz-offsite-proofs.timer hazync-lz-offsite-proofs-b2.timer     # proofs-46666b6f/
systemctl start hazync-lz-zapp-follow.timer                                         # after step 9's choice
systemctl start hazync-sponsor-bot.timer
```
The spine copies **only once the first genesis spine exists** (`/var/lib/hazync-lz/spine/spine.bin`; SWITCH.md 4c, c),
and then enabled, which they have never been: `systemctl enable --now hazync-lz-offsite-spine.timer hazync-lz-offsite-spine-b2.timer`.
Check: `systemctl list-timers 'hazync-*' 'libzync-*'` lists each; `systemctl --failed` is empty; the sponsor bot's
next run boots pods whose GPU smoke prints `MID=46666b6f…` (`journalctl -u hazync-sponsor-bot`).

## 11. The checks

```bash
cd /opt/hazync-lz && scripts/check-deployment.sh --local     # libzync section: every host, every pinned unit, the follower, no handover cap
systemctl start hazync-lz-check-spine.service hazync-lz-check-proofs.service libzync-xcheck.service
for u in hazync-lz-check-spine hazync-lz-check-proofs libzync-xcheck; do echo "$u $(systemctl show -p Result --value $u)"; done
scripts/check-unit-drift.sh localhost                        # "no drift": zz-handover.conf is gone
COORD_URL=https://api.hazync.org scripts/check-deployment.sh
curl -s https://api.hazync.org/api/meta
```
Check: all green, apart from what a fresh board cannot have yet (no spine: check-spine says so; xcheck checks only the
heights both runs have). Then a `hazync selftest` from a contributor machine with the v0.26.0 worker. The first
receipts to land: `ls /var/lib/hazync-lz/proofs | head`, and after the first offsite run
`journalctl -u hazync-lz-offsite-proofs -n 5 -o cat` names `proofs-46666b6f/`.

## 12. The website (hazync-web; the operator's step)

- `verify/hazync-verify.wasm` and `verify/hazync-verify.js` from the v0.26.0 release; `tools/check.py` `WASM_SHA` and
  `README.md` to the release's wasm sha (`12bbc9ae…` per `swiftsync-verify-wasm/README.md`; step 0.4 decides).
- `tools/verify-adapter-test.mjs` wants `imageId('mainnet')` to start with `8888d59c`: now `46666b6f` (the `8888d59c`
  fixtures still verify; the verifier keeps that image).
- `devs/index.html` image table: a `46666b6f… v0.26.0` row (undo commitment, board restarted from genesis); the
  release mentions in `index.html` and `how-it-works/index.html`; `assets/verdict.js`'s comment.
- **hazync-web #56** (the v1 board's contributors credited) deploys with this update.
- Check: `deploy/deploy.sh`'s own wasm sha check, then the explorer verifies the live spine in the browser.

## 13. The fleet

- **The web box's off-box spine check** (`hazync-web`), once the new board has a spine: install the release's
  `hazync-verify-x86_64-linux-gnu` as `/usr/local/bin/hazync-verify-lz` (the CORE `/usr/local/bin/hazync-verify` stays),
  `coordinator/check-spine.py` as `/usr/local/sbin/hazync-check-spine` (the installed copy predates libzync support),
  and `coordinator/deploy/watchdog/hazync-check-spine-remote.service` from this release; `daemon-reload`. Check:
  `systemctl start hazync-check-spine-remote.service` then its journal ends `The genesis proof holds.` (the same
  check with the v0.25.0 verifier and `8888d59c` passed against today's spine from the laptop, 2026-11-15).
- **Provers:** the v0.26.0 worker and host; older hosts are refused by `/api/meta`'s `method_id` pre-flight.
- **Pods:** the sponsor bot (step 10) boots them from the release.

## Record of the reset (fill in on the day)

| | |
|---|---|
| S (paper prep re-forested at) | |
| re-forest phase 2 time (rehearsal / real) | |
| `8888d59c` board at the reset: frontier, receipts, ranges rows, contributors carried | |
| first `46666b6f` spine | |

## Open questions for the owner

1. **Merge order.** This PR's units must be in the commit `/opt/hazync-lz` moves to (step 0.1). Merge it before
   v0.26.0 is tagged, or deploy the checkout at the merge commit after the tag.
2. **The zyncAPP's first facts** (step 9). No `46666b6f` proof of mainnet 1..200 exists; #732 adds one of block 1.
   Seed from it (Coin check works from the start, about one block), or wait for the board's first spine (no facts
   for the first hours)?
3. **The wasm the site pins.** `swiftsync-verify-wasm/README.md` pins `12bbc9ae…`; #732 reports `486ae6ed…` from a
   `build.sh` on hazync-coord. The release's `hazync-verify.wasm` decides (step 0.4); if it is neither, stop.
4. **Litestream.** A new replica path for the fresh ledger (`coordinator-lz-46666b6f-server1`) keeps the
   `8888d59c` replica as the record. Agreed, and is that replica kept for good?
5. **Start at boot.** `hazync-lz-coordinator`, `libzync-board-prep`, `libzync-board-fetch`, `libzync-prep` and
   `libzync-fetch` are `disabled` on the box: after a reboot the public board stays down. Intended?
6. **The paper prep's host.** The run's host is a `863c9ce3` build of 527 MB, beside several variants
   (`$P/bin/host_lz.*`); the released board host (v0.25.0) is 221 MB. Step 4 puts the release host in. If the
   next `PREP_PROGRESS` rate is well below 0.26 blocks/s, is a `46666b6f` build of the run's variant wanted?
7. **What carries.** Contributors only, as on 11-12; sponsorships and payments in the `8888d59c` ledger stay there.
