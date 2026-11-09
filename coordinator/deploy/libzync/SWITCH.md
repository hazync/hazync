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

- **Merged:** #677, #678, #679, #680, #682, #683, #685, #686, #687 and #688; then #676 and #684, which are drafts that land with the switch.
- **After #676 merges:** re-run `scripts/lineage.sh --write` and commit, because the squash changes the commit the row names.
- **Release v0.23.0** is cut and published with 5 assets. The pods and the Windows GUI download the libzync host from it.
- **`/opt/hazync-lz`** is at the merge commit, and `/opt/hazync-lz/bin/host` is the host from it (`method-id` = `863c9ce3…`). It must include #686's `HAZYNC_LZ_STATE_DIR` fallback.
- **The leaf pruner keeps `leaf_1.bin` for good.** The pods' GPU smoke (#685) and the worker's `selftest` use it.
- **The libzync board's own checks are green:**
  - `scripts/check-deployment.sh --local`, libzync section;
  - `libzync-xcheck`;
  - `check-proofs.py` with `HAZYNC_LIBZYNC=1` and `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out`.

## 2. Decisions for the operator (not made here)

The original board's database holds, as of 2026-11-09:

| table | rows |
|---|---|
| contributors | 13 |
| sponsorships | 3 |
| sponsor_payments | 2 |
| sponsor_keys | 3 |
| ranges | 244,016 |

The ranges are proofs under the CORE image (`37987b85`). They don't verify under libzync, so they don't carry over. Decide for each of the rest:

1. **Contributors (handles and keys).** Copy them into the libzync database so provers keep their identity, or start fresh?
2. **Open or paid sponsorships.** They name CORE block ranges. Refund, or re-issue them on the libzync board?
3. **The donations record and the ledger.** Keep the original database read-only as the record, served as history? Or migrate the rows?
4. **The original board's proofs, spine and offsite copies.** They stay where they are (`/var/lib/hazync`, `proofs-37987b85/` and `spine-37987b85/` in R2 and B2) as the record. Nothing is deleted.

## 3. The units

| unit(s) | at the switch |
|---|---|
| `hazync-coordinator` | **stop and disable.** Its port 8899 goes to the libzync coordinator; see step 4. |
| `hazync-lz-coordinator` | **becomes public.** Bind `0.0.0.0:8899`, and copy the original's drop-ins: `btcpay`, `zz-proxies`, `ratelimit`, `sponsor-bot`, `sponsor-open`, `zz-signed-claims`, `alert`, `hardening`, `umask`, `50-open-files…`. Add `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out`. Copy the files; never print them, they carry secrets. |
| `hazync-bridge`, `hazync-bridge-backfill` | **stop and disable.** They're CORE bundle producers; the board pass replaces them. |
| `hazync-archive-checkpoint{,-backfill}`, `hazync-offsite-checkpoints{,-b2}` | **disable.** These are CORE bridge rungs; a libzync re-start point is a prep save point. |
| `hazync-check-bundle-gap` | **retarget**: `HAZYNC_LIBZYNC=1`, `HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves` (already supported). |
| `hazync-prune-bundles` | **disable**; `hazync-lz-prune-leaves` replaces it. |
| `hazync-check-proofs` | **retarget** to `/var/lib/hazync-lz`, with `HAZYNC_HOST=/opt/hazync-lz/bin/host`, `HAZYNC_LIBZYNC=1`, `HAZYNC_BRIDGE_OUT=…/leaves` and `HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out` (#686). |
| `hazync-check-spine` | **retarget**: `COORD_DB` and `COORD_SPINE` to `/var/lib/hazync-lz`, plus `HAZYNC_LIBZYNC=1`, `HAZYNC_VERIFY=<swiftsync-verify from v0.23.0>` and `--method-id 863c9ce3…` (#688; it uses the libzync verifier's text output). Do the same for the web box's remote copy (`watchdog/hazync-check-spine-remote`). |
| `hazync-check-continuity`, `hazync-retention-check`, `hazync-coordinator-backup`, `hazync-offsite-ledger-b2`, `hazync-offsite-keys{,-b2}` | **retarget** `COORD_DB` and `COORD_PROOFS` to `/var/lib/hazync-lz`. These scripts call no host command. |
| `hazync-offsite-proofs{,-b2}`, `hazync-offsite-spine{,-b2}` | **disable.** The `hazync-lz-offsite-*` twins (#678, #687) replace them, and the CORE copies stay in the bucket. |
| `hazync-check-board-progress`, `hazync-check-breakers`, `hazync-check-disk`, `hazync-check-unit-drift`, `hazync-node-tip`, `hazync-offsite-watch`, `hazync-offsite-summary` | **keep.** Board-progress keeps `COORD_URL=http://127.0.0.1:8899`, which is the libzync coordinator after step 4. Unit-drift needs the new units in its list. |
| `hazync-sponsor-bot` | **retarget**: `HAZYNC_LIBZYNC=1` and `HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves` (#682, #685); release v0.23.0 for the pods. |
| `hazync-snapshot-emit`, `hazync-quiet-cutover` | **disable.** They're CORE and one-off. |
| `libzync-prep` (the paper run) | **keep** until it reaches the tip. |

## 4. Steps

1. **Freeze the original board.** Stop `hazync-sponsor-bot` and `hazync-coordinator`, then take a `VACUUM INTO` copy of its database for the record.
2. **Move the libzync coordinator to the public port.** Give `hazync-lz-coordinator` the original's drop-ins (step 3), `COORD_PORT=8899` and `COORD_BIND=0.0.0.0`. Then `daemon-reload` and restart it. Check `curl -s 127.0.0.1:8899/api/meta`: it should show `"libzync": true` and `method_id 863c9ce3…`.
3. **Swap the units.** Disable the CORE-only ones and retarget the rest, as in step 3.
4. **Enable the libzync timers:** `hazync-lz-offsite-{proofs,spine}{,-b2}` and the retargeted `hazync-check-proofs`.
5. **Check from outside.** `COORD_URL=https://api.hazync.org scripts/check-deployment.sh` and `--local` must both pass the libzync section. Then run a `hazync selftest` from a contributor machine against `api.hazync.org`.
6. **Switch the website** to the libzync board and the zAPP page. This is the operator's step.

## 5. Rollback

The original board's data is never changed, so going back is the reverse of steps 2 to 4:

1. Stop `hazync-lz-coordinator`, or move it back to `127.0.0.1:8900`.
2. Start `hazync-coordinator` and `hazync-sponsor-bot`.
3. Re-enable the disabled units and drop the retarget drop-ins.

Anything proved on the libzync board in the meantime stays in `/var/lib/hazync-lz`.

## 6. After the switch (not blockers)

- **Following the tip on libzync.** It needs prep running live at the tip, writing a leaf per new block, and a tip pipeline built for leaves. The current `tip_board.py`, `tip_runner.py`, `tip_smoke.py` and `sync_bundles.py` are CORE-only.
- **Retire the original board's code paths** once nothing calls them: the CORE bridge, the bundle tools and the bridge commands in the host.
