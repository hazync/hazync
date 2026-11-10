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
- **Release v0.23.0** is cut and published with exactly the 12 assets of `scripts/release-assets.txt`. The pods and the Windows GUI download the libzync host from it.
- **`/opt/hazync-lz`** is at the merge commit, and `/opt/hazync-lz/bin/host` is the host from it (`method-id` = `863c9ce3…`). It must include #686's `HAZYNC_LZ_STATE_DIR` fallback.
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
| `hazync-check-spine` | **retarget**: `COORD_DB` and `COORD_SPINE` to `/var/lib/hazync-lz`, plus `HAZYNC_LIBZYNC=1`, `HAZYNC_VERIFY=<swiftsync-verify from v0.23.0>` and `--method-id 863c9ce3…` (#688; it uses the libzync verifier's text output). Do the same for the web box's remote copy (`watchdog/hazync-check-spine-remote`). |
| `hazync-check-continuity`, `hazync-retention-check`, `hazync-coordinator-backup`, `hazync-offsite-ledger-b2`, `hazync-offsite-keys{,-b2}` | **retarget** `COORD_DB` and `COORD_PROOFS` to `/var/lib/hazync-lz`. These scripts call no host command. |
| `hazync-offsite-proofs{,-b2}`, `hazync-offsite-spine{,-b2}` | **disable.** The `hazync-lz-offsite-*` twins (#678, #687) replace them, and the CORE copies stay in the bucket. |
| `hazync-check-board-progress`, `hazync-check-breakers`, `hazync-check-disk`, `hazync-check-unit-drift`, `hazync-node-tip`, `hazync-offsite-watch`, `hazync-offsite-summary` | **keep.** Board-progress keeps `COORD_URL=http://127.0.0.1:8899`, which is the libzync coordinator after step 4. Unit-drift needs the new units in its list. |
| `hazync-sponsor-bot` | **retarget**: `HAZYNC_LIBZYNC=1` and `HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves` (#682, #685); release v0.23.0 for the pods. |
| `hazync-snapshot-emit`, `hazync-quiet-cutover` | **disable.** They're CORE and one-off. |
| `libzync-prep` (the paper run) | **keep** until it reaches the tip. |

## 4. Steps

1. **Freeze the original board.** Stop `hazync-sponsor-bot` and `hazync-coordinator`, then take a `VACUUM INTO` copy of its database for the record.
2. **Carry the contributors over**, with `hazync-lz-coordinator` stopped:
   ```
   carry-contributors.py --from /var/lib/hazync/coordinator.db --to /var/lib/hazync-lz/coordinator.db --apply
   ```
3. **Move the libzync coordinator to the public port.** Give `hazync-lz-coordinator` the original's drop-ins (step 3), `COORD_PORT=8899` and `COORD_BIND=0.0.0.0`. Then `daemon-reload` and restart it. Check `curl -s 127.0.0.1:8899/api/meta`: it should show `"libzync": true` and `method_id 863c9ce3…`.
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
