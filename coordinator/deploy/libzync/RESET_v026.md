# Board reset to 46666b6f (v0.26.0)

> ⏰ **NOT DONE YET.** This is the plan for the box `hazync-proof`, written on 2026-11-15 from the repo at `1b4ac3db`
> and a read-only survey of the box, and revised after a review of #733. Nothing here runs by itself: each step is
> done by hand, in order, and each ends with a check. A step whose check fails stops the reset; its rollback is under it.

Release v0.26.0 commits each block's undo data in the block forest ([`docs/libzync/UNDO.md`](../../../docs/libzync/UNDO.md)).
Its mainnet image `46666b6f…` has an **empty lineage**, so the public board **restarts from genesis** and every block
is proved again. The handover machinery (`LZ_HANDOVER_AT`, [`docs/HANDOVER.md`](../../../docs/HANDOVER.md)) stays in the code
for later upgrades and is switched **off** here. The paper prep run (`libzync-prep`) **also restarts from genesis**,
under the v0.26.0 host: the host before it never set the coin leaves' BIP30-tracked flag (bit 1) that the guest sets,
so every prep state from block 147,396 (the first tracked coinbase, `cec2a159…`) on disagrees with the proofs, and the
stores cannot go back to the 147,000 save point (the coin log is advanced in place, trie slots are reused, only the
two newest `.smt` are kept). Re-forest (UNDO.md §8) cannot repair it, because the coin leaves themselves change, so
it is not used. MEASURED on the old run: genesis to 147,000 in 10.5 min, then about 8 days to 611,000; the board
proves far slower than that, so prep stays ahead of it. The old run's files are moved aside, not deleted.

The previous reset, v1 to v2 under `8888d59c` on 2026-11-12, is [`SWITCH.md`](SWITCH.md) (step 0 and section 4c).
This page follows it wherever nothing has changed and says so.

## What the box held at the survey (2026-11-15, read-only)

| | |
|---|---|
| board | image `8888d59c`, frontier 1,999, 3,206 receipts, spine 1..1,999; `/api/meta` handover era `pre` at 2,000, so it has **not moved for about a day** (`blocked: 2000`); `/api/sponsors` lists no paid, proving or proven sponsorship |
| board pass | `libzync-board-prep` at 5,998, 5,000 leaf bundles (the window) |
| paper prep | `libzync-prep` at save point 607,000; 1,000 blocks in 3,842 s there (0.26 blocks/s, of which 403 s is the save); `out` = `/var/lib/libzync-fast/out`, 609,064 files, 2.4 GB; host image `863c9ce3` |
| zyncAPP | follower at 1,999 (`/srv/bulk/libzync-zapp`, host image `8888d59c`) |
| undo cache (NOT USED: re-forest dropped, see the top) | `hazync-coord:/root/undocommit/precompute/reforest/undo.tsv`: heights 1..620,000, 94 MB, sha256 `81f2532f6ee1557c6879e7bf4d2d3bcd54b042e83722b7453fca18217f6891fd` |
| disks | `/` (`/dev/vda4`, holds `/var/lib/libzync-fast`, `/var/lib/libzync-board`, `/var/lib/hazync-lz`): 1.5 TB free. `/srv/bulk` (`/dev/vdb`): 3.4 TB free |

**Found only on the box** (not in the repo, or different from it):

- `hazync-lz-coordinator.service.d/zz-handover.conf` (`LZ_HANDOVER_AT=2000`). #730 removed it from the repo. With the
  v0.26.0 host (no handover in its lineage) the coordinator would stay in the `pre` era and **cap the new board at
  1,999 for good**, every unit healthy (`server.py` `handover_era`). Step 6.1 removes it, with the old board already
  stopped; `check-deployment.sh --local` now fails a board capped that way and warns (SKIP) before it gets there.
- `/usr/local/sbin/hazync-lz-zapp-follow` predates #730: it cannot count `history.bin` records that carry an undo hash
  (bit 31). `/usr/local/sbin/hazync-lz-prune-leaves` and the installed `hazync-lz-prune-leaves.service` still default
  to port 8900 (the box's `zz-libzync.conf` drop-in covers it). Step 5 reinstalls all three.
- `/opt/hazync-lz/bin/zync-serve` is from 2026-11-09; it must be rebuilt from the release commit to read the new
  `history.bin` (it is not a release asset, `scripts/release.sh` step 3).
- `hazync-lz-offsite-spine{,-b2}.timer` are **disabled**: they failed on 11-12 on the empty spine and were never
  enabled afterwards (SWITCH.md step 5), so **the `8888d59c` spine was never copied off the box**. Step 1.3 copies it once.
- `hazync-lz-coordinator`, `libzync-board-prep`, `libzync-board-fetch`, `libzync-prep` and `libzync-fetch` are
  `disabled`: after a reboot the public board does not come back by itself. Left as they are (defaults, below).
- The web box's `hazync-check-spine-remote` still runs the CORE verifier (guest `37987b85`) and has failed every run
  since the 11-12 switch (444 in a row on 2026-11-15). Step 12 retargets it (the repo unit is fixed in #733).
- `check-unit-drift` on the box reports one drift today: `zz-handover.conf` / `LZ_HANDOVER_AT`. It goes with step 6.1.
  #733's unit changes change values only (`HAZYNC_METHOD_ID`, `--method-id`), which the drift check reports as a
  note, not drift, so installing them shows no false drift.

## Decisions (defaults; any of them can change)

| | 11-12 precedent (SWITCH.md) | here |
|---|---|---|
| coordinator data | `VACUUM INTO /root/lz-863c9ce3.db`, `mv /var/lib/hazync-lz /var/lib/hazync-lz.863c9ce3`, a fresh directory with its subdirectories | the same, named after `8888d59c`. The old directory stays read-only as the record |
| contributors | carried (`carry-contributors.py`): handle, key, first seen, last version; not block counts | **only contributors carry over**, from the `8888d59c` ledger |
| sponsorships, payments | stayed in the old ledger | the same. Donations come from BTCPay, not the ledger, so `/api/donations` is unaffected. Any `paid`, `proving` or `invoiced` row counted in step 0.2 must be settled before step 6 |
| offsite copies | new prefixes from the new id; the old ones kept, nothing deleted | `proofs-46666b6f/`, `spine-46666b6f/` (the units' `--method-id`); **the `8888d59c` prefixes, ledger and replica are kept as the record** |
| Litestream | v2 ledger added as `coordinator-lz-server1` | the fresh ledger under a NEW replica path `coordinator-lz-46666b6f-server1`, so it never meets the `8888d59c` replica, which stays |
| board pass | emptied (`rm`) and restarted from genesis | restarted from genesis; the old directories are **moved aside** (same filesystem, instant) **before the fresh coordinator first starts** (SWITCH.md step 0.4 before 0.6), so it never serves an old leaf or state. Re-foresting the pass is not possible: its leaves 2..998 are already pruned |
| paper prep | kept running | restarted from genesis under the v0.26.0 host (the BIP30-tracked flag bug made every state from 147,396 wrong; re-forest cannot repair it); the old run moved aside as `*.pre-v026` |
| zyncAPP facts | the 1..200 demo, then the follower at the spine | a FRESH `ZAPP_ROOT`; **it waits for the board's first spine** (no `46666b6f` proof of 1..200 exists; #732's block-1 receipt is the optional seed) |
| start at boot | n/a | **the five disabled units stay disabled**, as they are |
| website wasm pin | n/a | **the release asset's sha** (step 0.4) |
| `GENESIS_TIP` | `852c5c7e…` | unchanged: `libzync-genesis-tip` of the v0.26.0 mainnet host (hazync-coord build, 2026-11-15) is `852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167` |
| check-board-progress | n/a | nothing to do: a lower proven count is taken as a new board and a new baseline (#719) |

## Names used below

The ids of the images being retired are **read from the hosts that run them**, not typed: step 0.4 sets them, before
any host is replaced, and every name that carries one (`/var/lib/hazync-lz.<id>`, `host_lz.<id>`) uses its first 8.

```bash
N=46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c      # v0.26.0
REF=v0.26.0                                                               # or the merge commit after it (step 0.1)
R=/root/v026                                                              # the release's assets on the box
CLI="bitcoin-cli -datadir=/var/lib/hazync/bitcoin-client"                 # the box's node; the bridge units use this datadir
P=/srv/bulk/libzync-prep                                                  # the paper run's LZ_ROOT (out -> /var/lib/libzync-fast/out)
B=/srv/bulk/libzync-board                                                 # the board pass's LZ_ROOT
id_of() { "$1" method-id 2>/dev/null | grep -oE '[0-9a-f]{64}' | head -1; }
```

The shell variables do not survive a new ssh session: keep them in `/root/reset-v026.env` (step 0.4 writes it) and
`. /root/reset-v026.env` at the start of every session.

All commands run as root on `hazync-proof` unless a step names another machine. Never print a drop-in or config that
holds keys (`btcpay`, `sponsor-*`, `zz-proxies`, `zz-signed-claims`, `/etc/litestream.yml`, `/etc/hazync/backup/*`):
key names only.

## 0. Before anything stops

1. **#733 is in the commit `/opt/hazync-lz` moves to.** The box installs its units and scripts from that checkout
   (step 5). REF is the tag `v0.26.0` if the tag contains #733, else the merge commit after it. Check, on the laptop:
   `git show "$REF":coordinator/deploy/libzync/hazync-lz-check-spine.service | grep -c 46666b6f` prints 1, with REF set
   as in the names block above.
2. **The sponsor bot off, before the release is published.** It takes the `latest` release and boots pods whose host
   must match the board's `method_id`; between the release and step 6 they would not. Its unit also `Requires=` the
   coordinator, so a run during step 6 would start the coordinator.
   ```bash
   systemctl stop hazync-sponsor-bot.timer
   systemctl stop hazync-sponsor-bot.service            # Type=simple, SIGINT; it terminates its own pods (TimeoutStopSec=300)
   systemctl is-active hazync-sponsor-bot.timer hazync-sponsor-bot.service   # inactive, inactive
   SPONSOR_BOT_HOME=/var/lib/hazync/sponsor-bot python3 /opt/hazync-lz/coordinator/sponsor_bot.py stop-all   # terminates nothing
   test ! -e /var/lib/hazync/sponsor-bot/.failed && echo no-latch
   sqlite3 "file:/var/lib/hazync-lz/coordinator.db?mode=ro" "SELECT status, count(*) FROM sponsorships GROUP BY status"
   ```
   Check: both inactive; `stop-all` names no pod (one it names was left running: it is now terminated; note it in the
   record); `no-latch` (a latch written by this stop is cleared by hand, after the pod check); the sponsorship
   counts written into the record table at the end. ⛔ A `paid` or `proving` row would be stranded in the old ledger
   (the fresh one has no sponsorships, so the bot's `/api/bot/queue` is empty), and an `invoiced` one could still be
   paid into it: **stop and settle it** before step 6. Rollback: `systemctl start hazync-sponsor-bot.timer`.
3. **Release v0.26.0 is published** with the 13 assets of `scripts/release-assets.txt` and signed. Check on the laptop:
   `gh release view v0.26.0 -R hazync/hazync --json assets -q '.assets|length'` = 13. ⛔ `release.sh --from tag` can
   publish an EMPTY release and call it verified: count the assets.
4. **The release's assets on the box**, checked against the signed sums, and the retiring ids read from their hosts:
   ```bash
   mkdir -p $R && cd $R
   for f in hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu hazync-verify.wasm SHA256SUMS.txt SHA256SUMS.txt.asc; do
     curl -sfLO https://github.com/hazync/hazync/releases/download/v0.26.0/$f; done
   rm -rf $R/gnupg && cp -a /var/lib/hazync/sponsor-bot/gnupg $R/gnupg     # the keyring the bot verifies releases with, copied
   GNUPGHOME=$R/gnupg gpg --batch --verify SHA256SUMS.txt.asc SHA256SUMS.txt 2>&1 | grep -c 'Good signature'   # 1
   grep -E ' (hazync-host-x86_64-linux-gnu|hazync-verify-x86_64-linux-gnu|hazync-verify.wasm)$' SHA256SUMS.txt | sha256sum -c -
   chmod 755 hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu
   [ "$(id_of ./hazync-host-x86_64-linux-gnu)" = "$N" ] && echo new-host-ok
   ./hazync-host-x86_64-linux-gnu libzync-lineage | head -2    # HANDOVER none
   ./hazync-host-x86_64-linux-gnu libzync-genesis-tip | tail -1  # 852c5c7e…0167 (the unit's GENESIS_TIP)
   sha256sum hazync-verify.wasm                                  # the website pins this (step 11)
   O=$(id_of /opt/hazync-lz/bin/host); PO=$(id_of $P/bin/host_lz); ZO=$(id_of /srv/bulk/libzync-zapp/bin/host)
   o8=${O:0:8}; po8=${PO:0:8}; zo8=${ZO:0:8}; echo "board $o8  paper prep $po8  zyncAPP $zo8"
   declare -p N REF R CLI P B O PO ZO o8 po8 zo8 > /root/reset-v026.env; declare -f id_of >> /root/reset-v026.env
   ```
   Check: one `Good signature`; three `OK`; `new-host-ok`; `HANDOVER none`; the genesis tip; the three retiring ids
   are the survey's (board and zyncAPP the board's image, the paper prep its own) and all differ from `$N`. The wasm
   sha is what the site pins; `swiftsync-verify-wasm/README.md` says `12bbc9ae…` (#730) and #732 reports a local
   `build.sh` gave `486ae6ed…`: a release asset that is neither must be explained before step 11.
5. **zync-serve from the release commit**, built where cargo is (hazync-coord), copied through the laptop with
   literal paths (the laptop's shell has no `$R`):
   ```bash
   # on hazync-coord: a checkout at REF, at a path of your choice
   HZ_CHECKOUT=/root/hazync-v026; git -C $HZ_CHECKOUT checkout -q --detach v0.26.0   # or the merge commit (step 0.1)
   cargo build --release --locked --manifest-path $HZ_CHECKOUT/swiftsync-verify/Cargo.toml --bin zync-serve
   # on the laptop: the same checkout path, written out
   scp -3 hazync-coord:/root/hazync-v026/swiftsync-verify/target/release/zync-serve hazync-proof:/root/v026/zync-serve
   ```
   Check: `sha256sum` equal at both ends.
## 1. Stop the writers, and copy the old board's record off the box

1. **The paper prep and its block source.** Prep restarts from genesis (step 2), so it can stop at any moment; still,
   stop it between saves so the record left behind is a complete save point:
   ```bash
   journalctl -u libzync-prep -f -n0 -o cat | grep -m1 PREP_PROGRESS; systemctl stop libzync-prep libzync-fetch
   ```
   ESTIMATE: the wait is at most one save interval, about 64 min near 607,000 (3,842 s per 1,000 blocks, measured).
   Check: `systemctl is-active libzync-prep libzync-fetch` = `inactive inactive`. Rollback: `systemctl start
   libzync-fetch libzync-prep` (the old run carries on, still wrong from 147,396).
2. **The board's writers and the timers that would act on a half-reset board:**
   ```bash
   systemctl stop libzync-board-prep libzync-board-fetch
   systemctl stop hazync-lz-prune-leaves.timer hazync-lz-zapp-follow.timer libzync-xcheck.timer \
     hazync-lz-offsite-proofs.timer hazync-lz-offsite-proofs-b2.timer hazync-lz-offsite-spine.timer hazync-lz-offsite-spine-b2.timer \
     hazync-lz-check-spine.timer hazync-lz-check-proofs.timer hazync-check-board-progress.timer \
     hazync-check-bundle-gap.timer hazync-check-continuity.timer hazync-retention-check.timer \
     hazync-offsite-watch.timer hazync-offsite-summary.timer hazync-check-unit-drift.timer \
     hazync-coordinator-backup.timer hazync-offsite-ledger-b2.timer hazync-offsite-keys.timer hazync-offsite-keys-b2.timer
   ```
   ⛔ The offsite timers must stay stopped until step 9: from step 5 the units name `46666b6f`, the copy is
   append-only by name, and a receipt of the old board uploaded under `proofs-46666b6f/` would stay there for good.
   Check: `systemctl list-timers --all 'hazync-*' 'libzync-*'` shows each as inactive (n/a), and
   `systemctl list-units --state=activating,active 'hazync-lz-prune*' 'hazync-lz-zapp*' 'libzync-xcheck*' 'hazync-lz-offsite*'`
   is empty (a run in progress finishes first). The coordinator and zync-serve stay up (the board serves its 1,999,
   still capped by its handover drop-in, which stays until the board is stopped in step 6.1).
   Rollback: `systemctl start` the same list (the two spine timers were never running: leave them).
3. **The old board's last offsite copies**, with the units as they are (they name the old image):
   ```bash
   systemctl start hazync-lz-offsite-proofs.service hazync-lz-offsite-proofs-b2.service
   systemctl start hazync-lz-offsite-spine.service hazync-lz-offsite-spine-b2.service   # never ran on a real spine
   for u in hazync-lz-offsite-proofs hazync-lz-offsite-proofs-b2 hazync-lz-offsite-spine hazync-lz-offsite-spine-b2; do
     echo "$u $(systemctl show -p Result --value $u)"; journalctl -u $u -n 3 -o cat --no-pager; done
   ```
   Check: each `Result=success`; the spine copies name `spine-<the old image's 8>/… [1..1,999]` (R2 and B2). A failure
   here is not a reason to go on without the record: fix it first. Rollback: none needed (append-only copies).

## 2. The paper prep from genesis, under the v0.26.0 host

1. **Move the old run aside** (same filesystems, so each `mv` is instant; nothing is deleted until step 13):
   ```bash
   . /root/reset-v026.env; df -h / /srv/bulk
   O=$(readlink -f $P/out)                                     # /var/lib/libzync-fast/out
   mv $O $O.pre-v026 && mkdir $O
   for d in cl trie leaves blocks; do mv $P/$d $P/$d.pre-v026; done
   ls -A $O $P/cl $P/trie $P/leaves $P/blocks 2>&1 | grep -v '^ls:' | wc -l      # 0: nothing left in the run's dirs
   ```
   (`libzync-prep-run.sh` recreates `blocks cl trie out leaves`, finds no save point, and starts from genesis.)
2. **The v0.26.0 host**, keeping the old one by its image:
   ```bash
   cp -a $P/bin/host_lz $P/bin/host_lz.$po8
   install -m 755 $R/hazync-host-x86_64-linux-gnu $P/bin/host_lz
   [ "$(id_of $P/bin/host_lz)" = "$N" ] && echo prep-host-ok
   ```
   The v0.26.0 host derives the tracked flags itself (`prover/host/src/tracked.rs`) and refuses to continue from (or
   take a base step on) a saved state at or above mainnet 147,396, the first tracked coin, that lacks its
   `state_<h>.flags` marker, which only this host writes; so the old run cannot be picked up there by mistake. Below
   147,396 an older host's states are the same and are accepted. A leaf bundle at or above 147,396 whose block JSON has
   no `coinbase_flags` (cut by an older host) is refused too.
3. **Start the block source, then prep:**
   ```bash
   systemctl start libzync-fetch; sleep 60; ls $P/blocks | head -3            # block_1.json …
   systemctl start libzync-prep
   journalctl -u libzync-prep -n 20 -o cat --no-pager                          # "libzync-prep: 1..974936 from genesis"
   ```
   Check: `prep-host-ok`; prep starts from genesis; `HAZYNC_PREP_CHECK` (`$P/ref`, states 1..200) passes (any
   mismatch stops prep); the first `PREP_PROGRESS` lines come at the measured early rate (about 50 blocks/s near
   147,000 on the old run); after 147,396 the prep log names the tracked coins it flags. Rollback: `systemctl stop
   libzync-prep libzync-fetch`, `install -m 755 $P/bin/host_lz.$po8 $P/bin/host_lz`, empty the five dirs and `mv` each
   `*.pre-v026` back.

(Steps 3 and 4 were the re-forest of the paper run; they are gone, and the step numbers after them are kept so the
cross-references below stay right.)

## 5. `/opt/hazync-lz` at REF, and everything installed from it

The coordinator is still up on the old board (capped, its handover drop-in still in place).
```bash
DRY_RUN=1 UNIT=hazync-lz-coordinator REPO=/opt/hazync-lz /opt/hazync-lz/coordinator/deploy/deploy-coordinator.sh "$REF"
```
- **"served file UNCHANGED"** (`server.py` is unchanged from the deployed `e5ad283` to `1b4ac3db`): deploy now. The
  script does not restart the coordinator; it checks the unit is active and `/api/state` answers.
- **"served file CHANGES"** (REF is later than `1b4ac3db` and touched `server.py`): the deploy would restart the
  coordinator, on the OLD board. **Do not deploy now**: do this whole step at step 6.7 instead, where its restart is the
  fresh board's first start.

```bash
UNIT=hazync-lz-coordinator REPO=/opt/hazync-lz /opt/hazync-lz/coordinator/deploy/deploy-coordinator.sh "$REF"
cd /opt/hazync-lz/coordinator/deploy/libzync
install -m 644 *.service *.timer /etc/systemd/system/
install -m 755 hazync-lz-zapp-follow.sh    /usr/local/sbin/hazync-lz-zapp-follow
install -m 755 hazync-lz-prune-leaves.sh   /usr/local/sbin/hazync-lz-prune-leaves
install -m 755 hazync-lz-verify-spine.sh   /usr/local/sbin/hazync-lz-verify-spine
install -m 755 libzync-xcheck.py           /usr/local/sbin/libzync-xcheck
. /root/reset-v026.env
install -m 755 libzync-prep-run.sh $P/bin/libzync-prep-run.sh && install -m 755 libzync-prep-run.sh $B/bin/libzync-prep-run.sh
install -m 644 /opt/hazync-lz/coordinator/deploy/watchdog/hazync-check-spine-remote.service /root/   # for the web box, step 12
systemctl daemon-reload
```
(`*.service` includes the paper run's own `libzync-prep.service` / `libzync-fetch.service`, unchanged; their
settings are in drop-ins, which stay.) Check: `git -C /opt/hazync-lz rev-parse HEAD` is REF's commit
(`git rev-parse "$REF^{commit}"` on the laptop; `git describe --tags` says `v0.26.0` only when REF is the tag);
`grep -l "$O" /etc/systemd/system/hazync-lz-* /etc/systemd/system/libzync-*` prints nothing (SWITCH.md 4c, d);
`grep -c "$N" /etc/systemd/system/hazync-lz-check-spine.service` = 1; `sha256sum` of each installed script equals
the checkout's. Rollback: the same deploy command with `e5ad283` and the same installs from it, then `daemon-reload`.

## 6. Reset the board

1. **Stop it, and Litestream, then switch the handover off.** Removing the drop-in only now means no restart of the
   old board (`Restart=on-failure`, a deploy) can bring it back uncapped between its last offsite copy and its record:
   ```bash
   systemctl stop hazync-lz-coordinator hazync-lz-zync-serve litestream
   systemctl is-active hazync-lz-coordinator hazync-lz-zync-serve litestream        # inactive x3
   . /root/reset-v026.env
   mv /etc/systemd/system/hazync-lz-coordinator.service.d/zz-handover.conf /root/zz-handover.conf.$o8
   systemctl daemon-reload
   systemctl show hazync-lz-coordinator -p Environment --value | tr ' ' '\n' | grep -c LZ_HANDOVER_AT   # 0
   ```
2. **The record:**
   ```bash
   sqlite3 /var/lib/hazync-lz/coordinator.db "VACUUM INTO '/root/lz-$o8.db'"
   sqlite3 "file:/root/lz-$o8.db?mode=ro" 'PRAGMA integrity_check; SELECT count(*) FROM contributors; SELECT count(*) FROM ranges; SELECT status, count(*) FROM sponsorships GROUP BY status'
   mv /var/lib/hazync-lz /var/lib/hazync-lz.$o8
   ```
   Check: `ok`; the sponsorship counts are step 0.2's (if not, a payment landed meanwhile: settle it first); write the counts
   into the record table.
3. **A fresh directory, with every subdirectory** (SWITCH.md 4c, a), and the zyncAPP directory **empty**: the old
   board's facts are about an old-image proof and are not copied back (step 8):
   ```bash
   install -d -o hazync -g hazync /var/lib/hazync-lz /var/lib/hazync-lz/{proofs,spine,witnesses,coord_state,zapp}
   ```
4. **The old board pass out of the coordinator's sight, BEFORE its first start.** The coordinator serves the leaves in
   `HAZYNC_BRIDGE_OUT` (`server.py` `_servable_high`) and finds a range's start state in `HAZYNC_LZ_STATE_DIR`: left in
   place, the fresh board would offer the old pass's leaves (1..5,998, old forest) and their states to provers. The
   pass was stopped in step 1.2. The `out`, `cl`, `trie` symlinks under `$B` keep pointing at
   `/var/lib/libzync-board/*`, so the targets are moved and recreated, never the links:
   ```bash
   for d in out cl trie; do mv /var/lib/libzync-board/$d /var/lib/libzync-board/$d.$o8; install -d /var/lib/libzync-board/$d; done
   mv $B/leaves $B/leaves.$o8 && install -d $B/leaves
   rm -rf $B/blocks && install -d $B/blocks           # re-fetched; prep consumes them anyway
   rm -f /var/lib/hazync-checks/libzync-xcheck.last   # xcheck starts again at 1
   find /var/lib/libzync-board/out $B/leaves -mindepth 1 | grep -c .   # 0
   ```
5. **The new hosts and verifier** (each old one kept beside it):
   ```bash
   cd /opt/hazync-lz/bin   # untracked in the checkout
   cp -a host host.$o8 && cp -a hazync-verify hazync-verify.$o8 && cp -a zync-serve zync-serve.pre-v026
   install -m 755 $R/hazync-host-x86_64-linux-gnu host
   install -m 755 $R/hazync-verify-x86_64-linux-gnu hazync-verify
   install -m 755 $R/zync-serve zync-serve
   cp -a $B/bin/host_lz $B/bin/host_lz.$o8 && install -m 755 $R/hazync-host-x86_64-linux-gnu $B/bin/host_lz
   for h in /opt/hazync-lz/bin/host $B/bin/host_lz; do [ "$(id_of $h)" = "$N" ] && echo "$h ok"; done
   ```
6. **Litestream: the fresh ledger under a new replica path.** Edit `/etc/litestream.yml` in place without printing it:
   ```bash
   cp -p /etc/litestream.yml /root/litestream.yml.$o8
   sed -i 's#^\(\s*path:\s*\)coordinator-lz-server1\s*$#\1coordinator-lz-46666b6f-server1#' /etc/litestream.yml
   grep -cE '^\s*path:\s*coordinator-lz-46666b6f-server1\s*$' /etc/litestream.yml   # 1
   grep -cE '^\s*path:\s*coordinator-server1\s*$' /etc/litestream.yml               # 1 (v1, unchanged)
   ```
7. **The first start** (it creates the fresh database), **then the contributors carried** with it stopped. The first
   start is `systemctl start hazync-lz-coordinator`, or, if step 5 was put off ("served file CHANGES"), step 5's
   deploy command, whose restart starts it; then step 5's installs and `daemon-reload`.
   ```bash
   systemctl start hazync-lz-coordinator && sleep 10 && curl -s 127.0.0.1:8899/api/meta; echo
   systemctl stop hazync-lz-coordinator
   python3 /opt/hazync-lz/coordinator/deploy/libzync/carry-contributors.py --from /root/lz-$o8.db --to /var/lib/hazync-lz/coordinator.db
   python3 /opt/hazync-lz/coordinator/deploy/libzync/carry-contributors.py --from /root/lz-$o8.db --to /var/lib/hazync-lz/coordinator.db --apply
   chown hazync:hazync /var/lib/hazync-lz/coordinator.db*
   find /var/lib/hazync-lz -not -user hazync             # prints nothing
   systemctl start hazync-lz-coordinator
   ```
   Check: `/api/meta` = `{"method_id": "46666b6f…", "frontier": 0, "libzync": true, "handover": null, …}` ⛔ a
   `handover` that is not `null` means the drop-in is still there (6.1): stop. Until step 7 writes leaves there is
   nothing to claim. The carry plan names the old board's contributors; `/api/state` counts only those with work on
   THIS board, so it says 0 until someone proves (SWITCH.md 4b).
8. `systemctl start litestream`; check `journalctl -u litestream -n 20 -o cat` names
   `/var/lib/hazync-lz/coordinator.db` with no error.

**Rollback (the whole step):** stop the coordinator and Litestream;
`mv /var/lib/hazync-lz /var/lib/hazync-lz.46666b6f-failed && mv /var/lib/hazync-lz.$o8 /var/lib/hazync-lz`; the
`.$o8` / `.pre-v026` binaries back (both hosts, the verifier, zync-serve); the board pass's directories back (remove
the new empty ones, move the `.$o8` ones back); `/root/litestream.yml.$o8` back; the handover drop-in back
(`mv /root/zz-handover.conf.$o8 /etc/systemd/system/hazync-lz-coordinator.service.d/zz-handover.conf`); **step 5's
rollback** (the checkout and units back to `e5ad283`); `daemon-reload`; start the coordinator, zync-serve and Litestream.

## 7. The board pass from genesis under 46666b6f

1. **Start it** (the fetcher picks its start height once, at start, so it restarts too):
   `systemctl start libzync-board-fetch libzync-board-prep`.
2. **Check the stamp** (SWITCH.md step 0.7): block 1 starts from genesis, so look at the first state written:
   ```bash
   python3 -c 'import json;print(json.load(open("/var/lib/libzync-board/out/state_1.json"))["self_id"])'
   # [1869309510, 471612798, 2845418815, 153285052, 1779266528, 3091707695, 3338285247, 473752679]  (= 46666b6f… as 8 u32)
   head -c 8 $B/leaves/leaf_1.bin; echo                                                              # LZLEAF02
   ```
   and `journalctl -u libzync-board-prep -n 5 -o cat` says `1..974936 from genesis`.
Rollback: stop both; then step 6's rollback for the directories and host.

## 8. zync-serve and the zyncAPP follower, fresh

```bash
. /root/reset-v026.env
mv /srv/bulk/libzync-zapp /srv/bulk/libzync-zapp.$zo8
install -d /srv/bulk/libzync-zapp/bin && install -m 755 $R/hazync-host-x86_64-linux-gnu /srv/bulk/libzync-zapp/bin/host
```
zync-serve cannot start on an empty `/var/lib/hazync-lz/zapp`, and the follower does nothing until the board has a
spine. **The default is to wait** for the board's first genesis spine: the follower's timer (step 9) then preps 1..H
and starts zync-serve itself. Until then `/api/zync/*` answers 502 and Coin check says the fact service is
unreachable. On 11-12 the first spine reached block 344 in about 2.5 h with 4 pods (`RESULT_board_run_2026-11-12.txt`).

Optional, to have Coin check up from the start: seed from #732's real receipt of mainnet block 1
(`experimental/libzync-bench/zapp/mainnet_1_1_46666b6f.rcpt`, sha256 `35f527be…e5fd`; in the checkout when #732 is in
REF) by running the follower once on it. The follower's own test of a proof, `verify-any <it> 1` with the v0.26.0
mainnet host, gives `RANGE-OK lo=1 hi=1 … anchored=1` on it (hazync-coord's build, 2026-11-15); the rest of this path
is not tested:
```bash
install -d /root/zapp-seed && cp /opt/hazync-lz/experimental/libzync-bench/zapp/mainnet_1_1_46666b6f.rcpt /root/zapp-seed/spine.bin
COORD_SPINE=/root/zapp-seed ZAPP_ROOT=/srv/bulk/libzync-zapp ZAPP_SERVE_DIR=/var/lib/hazync-lz/zapp \
  ZAPP_FETCH=$B/bin/fetch_block_rpc.py HAZYNC_RPC_COOKIE=/var/lib/bitcoind/.cookie /usr/local/sbin/hazync-lz-zapp-follow
```
Check (either way, once it serves): the follower logs `serving facts at H`, `systemctl is-active hazync-lz-zync-serve`
= active, `curl -s 127.0.0.1:8601/tip` = `{"tip":H}`, and `curl -s https://api.hazync.org/api/zync/proof | sha256sum`
is the proof's. Rollback: stop zync-serve,
`rm -rf /srv/bulk/libzync-zapp && mv /srv/bulk/libzync-zapp.$zo8 /srv/bulk/libzync-zapp`, and put the old set back
from `/var/lib/hazync-lz.$o8/zapp` (only together with step 6's rollback: those facts are about the old board's proof).

## 9. The timers again

```bash
systemctl start hazync-lz-prune-leaves.timer hazync-lz-check-spine.timer hazync-lz-check-proofs.timer libzync-xcheck.timer \
  hazync-check-board-progress.timer hazync-check-bundle-gap.timer hazync-check-continuity.timer hazync-retention-check.timer \
  hazync-coordinator-backup.timer hazync-offsite-ledger-b2.timer hazync-offsite-keys.timer hazync-offsite-keys-b2.timer \
  hazync-offsite-watch.timer hazync-offsite-summary.timer hazync-check-unit-drift.timer
systemctl start hazync-lz-offsite-proofs.timer hazync-lz-offsite-proofs-b2.timer     # proofs-46666b6f/
systemctl start hazync-lz-zapp-follow.timer
systemctl start hazync-sponsor-bot.timer
```
The spine copies **only once the first genesis spine exists** (`/var/lib/hazync-lz/spine/spine.bin`; SWITCH.md 4c, c),
and then enabled, which they have never been: `systemctl enable --now hazync-lz-offsite-spine.timer hazync-lz-offsite-spine-b2.timer`.
Check:
- `systemctl list-timers 'hazync-*' 'libzync-*'` lists each; `systemctl --failed` is empty;
- **no handover**: `curl -s 127.0.0.1:8899/api/meta | jq .handover` = `null`, and
  `systemctl show hazync-lz-coordinator -p Environment --value | tr ' ' '\n' | grep -c LZ_HANDOVER_AT` = 0;
- **the sponsor bot**: `test ! -e /var/lib/hazync/sponsor-bot/.failed && echo no-latch`, and its next run
  (`journalctl -u hazync-sponsor-bot -n 20 -o cat`) ends successfully with nothing to do: the fresh ledger has no
  sponsorships, so `/api/bot/queue` is empty and it boots no pod. The first sponsorship on the new board is what
  makes it boot one, from the release (its GPU smoke must print `MID=46666b6f…`).

## 10. The checks

```bash
cd /opt/hazync-lz && scripts/check-deployment.sh --local     # libzync section: every host, every pinned unit, the follower, no handover line
systemctl start hazync-lz-check-spine.service hazync-lz-check-proofs.service libzync-xcheck.service
for u in hazync-lz-check-spine hazync-lz-check-proofs libzync-xcheck; do echo "$u $(systemctl show -p Result --value $u)"; done
scripts/check-unit-drift.sh localhost                        # "no drift": zz-handover.conf is gone
COORD_URL=https://api.hazync.org scripts/check-deployment.sh
curl -s https://api.hazync.org/api/meta
```
Check: all green, apart from what a fresh board cannot have yet (no spine: check-spine says so; xcheck checks only the
heights both runs have; the zyncAPP follower's host is checked, its facts come with the first spine). `--local` prints
no `LZ_HANDOVER_AT` line at all. Then a `hazync selftest` from a contributor machine with the v0.26.0 worker. The
first receipts to land: `ls /var/lib/hazync-lz/proofs | head`, and after the first offsite run
`journalctl -u hazync-lz-offsite-proofs -n 5 -o cat` names `proofs-46666b6f/`.

## 11. The website (hazync-web; the operator's step)

- `verify/hazync-verify.wasm` and `verify/hazync-verify.js` from the v0.26.0 release; `tools/check.py` `WASM_SHA` and
  `README.md` to **the release asset's sha** (step 0.4).
- `tools/verify-adapter-test.mjs` wants `imageId('mainnet')` to start with `8888d59c`: now `46666b6f` (the `8888d59c`
  fixtures still verify; the verifier keeps that image).
- `devs/index.html` image table: a `46666b6f… v0.26.0` row (undo commitment, board restarted from genesis); the
  release mentions in `index.html` and `how-it-works/index.html`; `assets/verdict.js`'s comment.
- **hazync-web #56** (the v1 board's contributors credited) deploys with this update.
- Check: `deploy/deploy.sh`'s own wasm sha check, then the explorer verifies the live spine in the browser.

## 12. The fleet

- **The web box's off-box spine check** (`hazync-web`), once the new board has a spine: install the release's
  `hazync-verify-x86_64-linux-gnu` as `/usr/local/bin/hazync-verify-lz` (the CORE `/usr/local/bin/hazync-verify` stays),
  `coordinator/check-spine.py` as `/usr/local/sbin/hazync-check-spine` (the installed copy predates libzync support),
  and `coordinator/deploy/watchdog/hazync-check-spine-remote.service` from REF; `daemon-reload`. Check:
  `systemctl start hazync-check-spine-remote.service` then its journal ends `The genesis proof holds.` (the same
  check with the v0.25.0 verifier and its image passed against today's spine from the laptop, 2026-11-15).
- **Provers:** the v0.26.0 worker and host; older hosts are refused by `/api/meta`'s `method_id` pre-flight.
- **Pods:** the sponsor bot (step 9) boots them from the release, once there is a sponsorship.

## 13. Clean-up (after the new board has run for a while)

Everything kept for a rollback, with where it lives (`du -sh` each first):

| what | where | filesystem |
|---|---|---|
| the old paper run | `/var/lib/libzync-fast/out.pre-v026`, `$P/{cl,trie,leaves,blocks}.pre-v026` (the coin log and trie are hundreds of GB) | `/` and `/srv/bulk` |
| the old hosts | `$P/bin/host_lz.$po8`, `/opt/hazync-lz/bin/{host,hazync-verify}.$o8`, `zync-serve.pre-v026`, `$B/bin/host_lz.$o8` | `/srv/bulk`, `/` |
| the old board pass | `/var/lib/libzync-board/{out,cl,trie}.$o8`, `$B/leaves.$o8` | `/`, `/srv/bulk` |
| the old zyncAPP follower | `/srv/bulk/libzync-zapp.$zo8` | `/srv/bulk` |
| staged inputs | `$R`, `/root/undo-*.tsv`, `/root/zapp-seed` | `/` |

**Kept for good** (the record; never deleted): `/var/lib/hazync-lz.$o8`, `/root/lz-$o8.db`,
`/root/zz-handover.conf.$o8`, `/root/litestream.yml.$o8`, and the old prefixes and replica off the box.
The board pass stops at its free-space floor `HAZYNC_PREP_MIN_FREE_GB=250` (the paper run's is 200) on the coin log's
filesystem, `/` for both: check `df -h /` against those before and after.

## Record of the reset (fill in on the day)

| | |
|---|---|
| sponsorships by status in the old ledger (step 0.2 / 6.2) | |
| paper prep restart: time from genesis to 147,396, and the tracked coins it flagged (vs `reproduce/libzync-tracked.json`) | |
| the old board at the reset: frontier, receipts, ranges rows, contributors carried | |
| new prep rate after the swap | |
| first `46666b6f` spine | |

## Defaults and open questions

**Defaults** (the reset follows these unless decided otherwise):

1. Only contributors carry over (11-12 precedent); sponsorships and payments stay in the old ledger.
2. The old offsite prefixes, the old ledger (directory and `VACUUM INTO` copy) and its Litestream replica are kept as
   the record; the fresh ledger replicates under `coordinator-lz-46666b6f-server1`.
3. The zyncAPP waits for the board's first spine rather than seeding from #732's block-1 receipt.
4. The five boot-disabled units (`hazync-lz-coordinator`, `libzync-board-prep`, `libzync-board-fetch`, `libzync-prep`,
   `libzync-fetch`) stay as they are.
5. The website pins the release asset's wasm sha.

**Open:**

1. **Merge order.** #733's units must be in REF (step 0.1): merge it before v0.26.0 is tagged, or deploy the checkout
   at the merge commit after the tag.
2. **Stranded sponsorships.** Any `paid`, `proving` or `invoiced` row step 0.2 counts in the old ledger: refund, carry
   by hand, or leave? (`/api/sponsors` showed none paid, proving or proven on 2026-11-15.)
3. **The paper prep's host.** The run's host is an old-image build of 527 MB, beside several variants
   (`$P/bin/host_lz.*`); the released board host (v0.25.0) is 221 MB. Step 4 puts the release host in. If the next
   `PREP_PROGRESS` rate is well below 0.26 blocks/s, is a `46666b6f` build of the run's variant wanted?
4. **The wasm sha**, if the release asset's is neither `12bbc9ae…` (README) nor `486ae6ed…` (#732's local build).
