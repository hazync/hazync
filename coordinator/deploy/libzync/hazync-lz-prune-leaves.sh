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
