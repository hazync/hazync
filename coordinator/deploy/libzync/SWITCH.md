# Switching the board to libzync

The libzync board runs beside the original one on the coordinator box until the switch.

| | libzync board |
|---|---|
| coordinator | `hazync-lz-coordinator` on `127.0.0.1:8900`, with its own database, proofs and spine under `/var/lib/hazync-lz` |
| bundles | the paced board pass `libzync-board-prep`, writing to `/srv/bulk/libzync-board/leaves` |
| leaf pruner | `hazync-lz-prune-leaves` |
| prep cross-check | `libzync-xcheck` |
| offsite copies | `hazync-lz-offsite-{proofs,spine}{,-b2}` |

The switch makes it the public board and retires the original.

Nothing here runs by itself. Each step is done by hand, in order, and step 1 must be complete first.

## 1. Before the switch

All of these must hold:

- **Merged:** #677, #678, #679, #680, #682, #683, #685, #686, #687, #688, #690 and #691; then #676 and #684, which are drafts that land with the switch.
- **After #676 merges:** re-run `scripts/lineage.sh --write` and commit, because the squash changes the commit the row names.
- **Release v0.24.0** (risc0 3.1.0, image `8888d59c`) is cut and published with exactly the 12 assets of `scripts/release-assets.txt`. The pods and the Windows GUI download the libzync host from it. v0.23.0 (`863c9ce3`, risc0 3.0.5) is withdrawn: its proofs are refused by 3.1.0 verifiers.
- **The board pass has been restarted under `8888d59c`** (step 0 below).
- **`/opt/hazync-lz`** is at the merge commit, and `/opt/hazync-lz/bin/host` is the host from it (`method-id` = `8888d59c…`). It must include #686's `HAZYNC_LZ_STATE_DIR` fallback.
- **The leaf pruner keeps `leaf_1.bin` for good.** The pods' GPU smoke (#685) and the worker's `selftest` use it.
- **The libzync board's own checks are green:**
  - `scripts/check-deployment.sh --local`, libzync section;
  - `libzync-xcheck`;
  - `check-proofs.py` with `HAZYNC_LIBZYNC=1` and `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out`.

## 2. Decisions (made by the operator on 2026-11-09)

The original board's database held, on 2026-11-09:

| table | rows |
|---|---|
| contributors | 13 |
| sponsorships | 3 |
| sponsor_payments | 2 |
| sponsor_keys | 3 |
| ranges | 244,016 |

- **Contributors carry over**, by `carry-contributors.py` (#691): handle, key, first seen and last version. Block counts don't, because they count CORE proofs.
- **Nothing else carries over.** Sponsorships, sponsor payments and the donations record stay in the original database, which is kept read-only as the record.
- **The CORE proofs** (244,016 ranges under `37987b85`) don't verify under libzync. They stay where they are as the record: `/var/lib/hazync`, and `proofs-37987b85/` and `spine-37987b85/` in R2 and B2. Nothing is deleted.
- **Release v0.23.0** keeps the asset names: `hazync-verify-*` and `hazync-verify.wasm` now contain the libzync verifier. It adds the Windows CUDA prover, and ships nothing extra for CORE proofs, since v0.22.1 keeps its CORE verifier.

## 3. The units

| unit(s) | at the switch |
|---|---|
| `hazync-coordinator` | **stop and disable.** Its port 8899 goes to the libzync coordinator; see step 4. |
| `hazync-lz-coordinator` | **becomes public.** Bind `0.0.0.0:8899`, and copy the original's drop-ins: `btcpay`, `zz-proxies`, `ratelimit`, `sponsor-bot`, `sponsor-open`, `zz-signed-claims`, `alert`, `hardening`, `umask`, `50-open-files…`. Add `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out`. Copy the files; never print them, they carry secrets. ⛔ **Never copy `paths-and-user.conf` or `network.conf`:** the first points `COORD_DB`, `COORD_PROOFS`, `HAZYNC_HOST` and `ReadWritePaths` at the CORE board and would override the libzync paths; the second's `TRUSTED_PROXIES` is set again by `zz-proxies.conf`, and its `COORD_BIND` is replaced here. `HAZYNC_LZ_STATE_DIR` is already in the libzync unit's `state-dir.conf`. |
| `hazync-bridge`, `hazync-bridge-backfill` | **stop and disable.** They're CORE bundle producers; the board pass replaces them. |
| `hazync-archive-checkpoint{,-backfill}`, `hazync-offsite-checkpoints{,-b2}` | **disable.** These are CORE bridge rungs; a libzync re-start point is a prep save point. |
| `hazync-check-bundle-gap` | **retarget**: `HAZYNC_LIBZYNC=1`, `HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves` (already supported). |
| `hazync-prune-bundles` | **disable**; `hazync-lz-prune-leaves` replaces it. |
| `hazync-check-proofs` | **retarget** to `/var/lib/hazync-lz`, with `HAZYNC_HOST=/opt/hazync-lz/bin/host`, `HAZYNC_LIBZYNC=1`, `HAZYNC_BRIDGE_OUT=…/leaves` and `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out` (#686). |
| `hazync-check-spine` | **retarget**: `COORD_DB` and `COORD_SPINE` to `/var/lib/hazync-lz`, plus `HAZYNC_LIBZYNC=1`, `HAZYNC_VERIFY=<swiftsync-verify from v0.24.0>` and `--method-id 8888d59c…` (#688; it uses the libzync verifier's text output). Do the same for the web box's remote copy (`watchdog/hazync-check-spine-remote`). |
| `hazync-check-continuity`, `hazync-retention-check`, `hazync-coordinator-backup`, `hazync-offsite-ledger-b2`, `hazync-offsite-keys{,-b2}` | **retarget** `COORD_DB` and `COORD_PROOFS` to `/var/lib/hazync-lz`. These scripts call no host command. |
| `hazync-offsite-proofs{,-b2}`, `hazync-offsite-spine{,-b2}` | **disable.** The `hazync-lz-offsite-*` twins (#678, #687) replace them, and the CORE copies stay in the bucket. |
| `hazync-check-board-progress`, `hazync-check-breakers`, `hazync-check-disk`, `hazync-check-unit-drift`, `hazync-node-tip`, `hazync-offsite-watch`, `hazync-offsite-summary` | **keep.** Board-progress keeps `COORD_URL=http://127.0.0.1:8899`, which is the libzync coordinator after step 4. Unit-drift needs the new units in its list. |
| `hazync-sponsor-bot` | **retarget**: `HAZYNC_LIBZYNC=1` and `HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves` (#682, #685); release v0.24.0 for the pods (v0.23.0 was decided on 11-09 and withdrawn on 11-11). |
| `hazync-snapshot-emit`, `hazync-quiet-cutover` | **disable.** They're CORE and one-off. |
| `libzync-prep` (the paper run) | **keep** until it reaches the tip. |

## 4. Steps

0. **Restart the libzync board pass under `8888d59c`** (needed once, because v0.23.0's image was replaced; 11-11).
   Every leaf bundle carries `prev`, the state before its block, stamped with the image that prepared it (`prep.rs`).
   A leaf's anchor is the link digest of that `prev`, and the link digest covers `self_id`. So leaves proved by
   `8888d59c` from bundles prepared under `863c9ce3` each verify on their own but never chain: no join succeeds and
   the frontier stops. The re-stamp on a continued prep fixes only the run's starting state, not bundles already
   written. On 11-11 the pass held 5,000 bundles (blocks 1..5,000) and the libzync coordinator 5 proofs of blocks 1..4
   and 1 contributor, so nothing of value is lost:
   The board pass's `out`, `cl` and `trie` under `/srv/bulk/libzync-board` are **symlinks** into
   `/var/lib/libzync-board` (the coordinator, check-proofs and xcheck read that path): empty the targets, never the
   links. Prep consumes its block files (`HAZYNC_PREP_CONSUME=1`), and the fetcher picks its start height once at
   startup, so the fetcher must restart too or prep waits for `block_1.json` for ever.
   1. Stop `libzync-board-prep`, `libzync-board-fetch`, `hazync-lz-coordinator`, `hazync-lz-zync-serve`, the timers
      `hazync-lz-offsite-{proofs,spine}{,-b2}.timer`, `hazync-lz-prune-leaves.timer`, `hazync-lz-check-{proofs,spine}.timer`
      and `libzync-xcheck.timer`, and any leaf workers.
   2. Keep a record: `sqlite3 /var/lib/hazync-lz/coordinator.db "VACUUM INTO '/root/lz-863c9ce3.db'"`, then
      `mv /var/lib/hazync-lz /var/lib/hazync-lz.863c9ce3`.
   3. Recreate it for the `hazync` user and put the zApp data back (zync-serve reads it):
      `install -d -o hazync -g hazync /var/lib/hazync-lz && cp -a /var/lib/hazync-lz.863c9ce3/zapp /var/lib/hazync-lz/`.
   4. Empty the pass: `rm -rf /var/lib/libzync-board/{out,cl,trie}/* /srv/bulk/libzync-board/leaves/* /srv/bulk/libzync-board/blocks/*`
      (the symlinks stay), and `rm -f /var/lib/hazync-checks/libzync-xcheck.last` so xcheck starts again at 1.
   5. Install the v0.24.0 host as **both** `/opt/hazync-lz/bin/host` and `/srv/bulk/libzync-board/bin/host_lz` (the
      pass runs the second, `libzync-prep-run.sh`). `host_lz method-id` must print `8888d59c…`.
   6. Start `libzync-board-fetch`, then `libzync-board-prep`, then `hazync-lz-coordinator` (it creates a fresh
      database, which step 2's `carry-contributors.py` needs, since it opens the target `mode=rw`) and
      `hazync-lz-zync-serve`; re-enable the prune, check and xcheck timers (not the offsite ones yet).
   7. Check the stamp. Block 1's bundle starts from genesis, whose `self_id` is zero, so look at the first
      state written: `self_id` in `/var/lib/libzync-board/out/state_1.json` must be
      `[2631239816, 2674494539, 4198157524, 4069463800, 1375117754, 2714639643, 453263298, 4095025963]` (= `8888d59c…`
      as eight u32 words; it is `leaf_2.bin`'s `prev`).
   Only then carry on with step 1. ⛔ Do not re-enable the `hazync-lz-offsite-*` timers before step 5 of section 4: the copy is
   append-only by name, so a `863c9ce3` receipt uploaded under `proofs-8888d59c/` would stay there, and the
   re-proved receipt of the same range would never be uploaded.
1. **Freeze the original board.** Stop `hazync-sponsor-bot` and `hazync-coordinator`, then take a `VACUUM INTO` copy of its database for the record.
2. **Carry the contributors over**, with `hazync-lz-coordinator` stopped:
   ```
   carry-contributors.py --from /var/lib/hazync/coordinator.db --to /var/lib/hazync-lz/coordinator.db --apply
   ```
3. **Move the libzync coordinator to the public port.** Give `hazync-lz-coordinator` the original's drop-ins (step 3), `COORD_PORT=8899` and `COORD_BIND=0.0.0.0`. Then `daemon-reload` and restart it. Check `curl -s 127.0.0.1:8899/api/meta`: it should show `"libzync": true` and `method_id 8888d59c…` (risc0 3.1.0; the 11-10 rehearsal below ran under 863c9ce3).
4. **Swap the units.** Disable the CORE-only ones and retarget the rest, as in step 3.
5. **Enable the libzync timers:** `hazync-lz-offsite-{proofs,spine}{,-b2}` and the retargeted `hazync-check-proofs`.
6. **Check from outside.** `COORD_URL=https://api.hazync.org scripts/check-deployment.sh` and `--local` must both pass the libzync section. Then run a `hazync selftest` from a contributor machine against `api.hazync.org`.
7. **Switch the website** to the libzync board and the zAPP page. This is the operator's step.

## 4b. Rehearsed (2026-11-10)

Steps 2 and 3 were run on a temporary `hazync-lz-rehearse` unit:
- **Setup:** port 8901, bound to 127.0.0.1, with `VACUUM INTO` copies of both databases and the 10 copied drop-ins. A last drop-in redirected the data paths and cleared `OnFailure`.
- **Step 3 (drop-ins):**
  - it started with the copied drop-ins;
  - `/api/meta` gave `libzync: true`, `863c9ce3…`, frontier 4;
  - the journal showed no errors. `hardening.conf`'s `ProtectSystem=full` leaves `/var/lib/hazync-lz` writable.
- **Step 2 (contributors):** `carry-contributors.py --apply` copied 13 contributors; 14 in all.
  - `/api/state`'s `contributors` still says **1**. That's expected: it counts contributors with work on THIS board, so the site shows 1 prover until the carried ones prove something.
- **Clean-up:** the unit and its data were removed afterwards. Both real boards were untouched throughout.

To repeat it before the real switch, follow the same pattern: a copy of the unit, its drop-ins plus a `zzz-` override of the port and data paths, and copies of the data.

## 5. Rollback

The original board's data is never changed, so going back is the reverse of steps 3 to 5:

1. Stop `hazync-lz-coordinator`, or move it back to `127.0.0.1:8900`.
2. Start `hazync-coordinator` and `hazync-sponsor-bot`.
3. Re-enable the disabled units and drop the retarget drop-ins.

Anything proved on the libzync board in the meantime stays in `/var/lib/hazync-lz`.

## 6. After the switch (not blockers)

- **Following the tip on libzync.** It needs prep running live at the tip, writing a leaf per new block, and a tip pipeline built for leaves. The current `tip_board.py`, `tip_runner.py`, `tip_smoke.py` and `sync_bundles.py` are CORE-only.
- **Retire the original board's code paths** once nothing calls them: the CORE bridge, the bundle tools and the bridge commands in the host.
