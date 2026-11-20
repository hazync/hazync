#!/usr/bin/env bash
# The libzync board's leaf pruner: deletes prep's leaf_<n>.bin once block n is inside the genesis-anchored proven
# range (the coordinator's frontier) and at least MARGIN blocks below it. The board pass (libzync-board-prep) pauses
# while HAZYNC_PREP_LEAF_WINDOW bundles are waiting, so this is what lets it go on.
#
# Why below the frontier is safe: a receipt the coordinator still has to verify starts above the frontier (a range
# starting at 1 is checked against genesis, not a file), so no verify-any needs a leaf at or below it; and a leaf at
# or below it is proved, its receipt held in the coordinator's proofs. A block whose receipt were lost would need its
# bundle again: the board pass's saved states can regenerate it (libzync-prep from the save point below it).
#
# Exit codes follow the check contract: 0 done, 2 could not check (coordinator unreachable or no frontier).
#   LEAVES   leaf bundle directory (default /srv/bulk/libzync-board/leaves)
#   COORD    the libzync coordinator (default http://127.0.0.1:8899, the public port since the switch)
#   MARGIN   blocks below the frontier to keep (default 1000)
#   KEEP     leaf bundles kept for good, comma-separated heights (default 1: block 1's bundle is what the worker's
#            `selftest` and the sponsor bot's pod GPU smoke prove, a few KB)
#   TIP_LEAVES  the tip run's bundle directory (D1, docs/libzync/REORG.md §19; unset: no tip run). Its bundles sit ABOVE
#            the frontier until the backfill reaches the handover height X (`tip_run.from` in /api/meta), so the rule
#            above never frees them: a tip bundle goes once its block is PROVEN (the coordinator's /api/blockstatus) and
#            at least TIP_DEPTH (6) blocks below the node's tip (NODE_TIP_FILE, /var/lib/hazync/node_tip; missing or
#            stale: none go). Its sample bundles below X stay. A proven block needs its bundle no more: verify-any reads
#            the run's state files, and a reorg's rewind moves what is there and re-cuts the new branch's.
#   --apply  delete (without it: report what would go)
set -uo pipefail
LEAVES="${LEAVES:-/srv/bulk/libzync-board/leaves}"
COORD="${COORD:-http://127.0.0.1:8899}"
MARGIN="${MARGIN:-1000}"
KEEP="${KEEP-1}"
APPLY=0; [ "${1:-}" = --apply ] && APPLY=1

meta=$(curl -sf --max-time 20 "$COORD/api/meta") || { echo "[prune-leaves] cannot reach $COORD/api/meta" >&2; exit 2; }
F=$(printf '%s' "$meta" | python3 -c 'import json,sys; f=json.load(sys.stdin).get("frontier"); print(f if isinstance(f,int) else "")')
[ -n "$F" ] || { echo "[prune-leaves] no integer frontier in $COORD/api/meta" >&2; exit 2; }
CUT=$((F - MARGIN))
if [ "$CUT" -lt 1 ]; then echo "[prune-leaves] frontier $F, margin $MARGIN: nothing below the cut"; exit 0; fi
n=0; bytes=0
while IFS= read -r f; do
    h=${f#leaf_}; h=${h%.bin}
    case "$h" in ''|*[!0-9]*) continue;; esac
    [ "$h" -le "$CUT" ] || continue
    case ",$KEEP," in *",$h,"*) continue;; esac
    n=$((n + 1)); bytes=$((bytes + $(stat -c %s "$LEAVES/$f")))
    [ "$APPLY" = 1 ] && rm -f -- "$LEAVES/$f"
done < <(find "$LEAVES" -maxdepth 1 -type f -name 'leaf_*.bin' -printf '%f\n')
echo "[prune-leaves] frontier $F, margin $MARGIN: $([ "$APPLY" = 1 ] && echo deleted || echo would delete) $n leaf bundles ($((bytes / 1048576)) MB) at or below $CUT"

# the tip run's bundles (D1)
[ -n "${TIP_LEAVES:-}" ] || exit 0
X=$(printf '%s' "$meta" | python3 -c 'import json,sys; t=json.load(sys.stdin).get("tip_run") or {}; f=t.get("from"); print(f if isinstance(f,int) else "")')
[ -n "$X" ] || { echo "[prune-leaves] TIP_LEAVES is set but the coordinator reports no tip run (tip_run.from): tip bundles left as they are"; exit 0; }
NT_FILE="${NODE_TIP_FILE:-/var/lib/hazync/node_tip}"
NT=$(find "$NT_FILE" -mmin -60 -exec cat {} \; 2>/dev/null | tr -dc 0-9)
[ -n "$NT" ] || { echo "[prune-leaves] no fresh node tip in $NT_FILE: no tip bundle deleted" >&2; exit 2; }
TCUT=$((NT - ${TIP_DEPTH:-6}))
status=$(curl -sf --max-time 60 "$COORD/api/blockstatus") || { echo "[prune-leaves] cannot reach $COORD/api/blockstatus" >&2; exit 2; }
# the proven heights among the tip run's bundles (blockstatus runs: [lo, hi, status], 3 proven / 4 folded / 5 anchored)
gone=$(printf '%s' "$status" | python3 -c '
import json, os, sys
x, cut, d = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
runs = json.load(sys.stdin)["runs"]
have = sorted(int(n[5:-4]) for n in os.listdir(d) if n.startswith("leaf_") and n.endswith(".bin") and n[5:-4].isdigit())
for h in have:
    if x <= h <= cut and any(lo <= h <= hi and st >= 3 for lo, hi, st in runs):
        print(h)' "$X" "$TCUT" "$TIP_LEAVES") || { echo "[prune-leaves] could not read the tip bundles or the block status" >&2; exit 2; }
n=0; bytes=0
for h in $gone; do
    f="$TIP_LEAVES/leaf_$h.bin"
    [ -e "$f" ] || continue
    n=$((n + 1)); bytes=$((bytes + $(stat -c %s "$f")))
    [ "$APPLY" = 1 ] && rm -f -- "$f"
done
echo "[prune-leaves] tip run from $X, node tip $NT: $([ "$APPLY" = 1 ] && echo deleted || echo would delete) $n proven tip bundles ($((bytes / 1048576)) MB) at or below $TCUT"
