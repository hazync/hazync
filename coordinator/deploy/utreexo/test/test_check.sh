#!/bin/bash
# hazync-check-utreexo-follow on every path, with a stub node and a stub zyncapp (seconds, no block files):
#   test_check.sh             the check's verdicts (0 holds, 1 failing, 2 could not check)
#   test_check.sh --control   the same check with its stall comparison removed: the stalled case must be MISSED (exit 0),
#                             which shows the stall case below can tell a check that has the guard from one that has not
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); C=$HERE/../hazync-check-utreexo-follow.sh
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
mkdir -p "$W/d/pub" "$W/bin"
if [ "${1:-}" = --control ]; then
  python3 - "$C" "$W/check" <<'PY'
import sys
s = open(sys.argv[1]).read()
a = 'if [ "$h" -lt "$target" ] && [ "$age" -gt "$STALL_SECS" ]; then'
assert s.count(a) == 1
open(sys.argv[2], 'w').write(s.replace(a, 'if false; then'))
PY
  chmod +x "$W/check"; C=$W/check
fi
printf '#!/bin/bash\n[ -n "${STUB_RPC_FAIL:-}" ] && exit 28\n[ "$1" = getblockcount ] && echo "$STUB_TIP"\n' > "$W/bin/cli"
printf '#!/bin/bash\n[ -n "${STUB_CORE_FAIL:-}" ] && exit 1\necho "CORE height=$STUB_T tip=%s image=%s"\n' "$(printf 'a%.0s' $(seq 64))" "$(printf '0%.0s' $(seq 64))" > "$W/bin/zyncapp"
chmod +x "$W/bin"/*
export UTREEXO_DIR=$W/d ZYNCAPP=$W/bin/zyncapp CORE=/dev/null BITCOIN_CLI=$W/bin/cli MARGIN=6 UTREEXO_STALL_SECS=3600 \
       FOREST_HEAP_WARN_BYTES=1000 STUB_TIP=1000 STUB_T=500
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
pub () { echo "{\"height\": $1}" > "$W/d/pub/latest.json"; touch -d "@$(( $(date +%s) - $2 ))" "$W/d/pub/latest.json"; }
run () { "$C" > "$W/out" 2>&1; echo $?; }

if [ "$C" = "$W/check" ]; then
  pub 400 7200
  t "control: the check WITHOUT its stall comparison misses a stall (published 400 of 500, 2 h old)" 0 "$(run)"
  [ "$fails" -eq 0 ] && echo UTREEXO_CHECK_CONTROL_OK || { echo "UTREEXO_CHECK_CONTROL_FAILED"; exit 1; }
  exit 0
fi

t "nothing published is failing" 1 "$(run)"
pub 500 7200; t "caught up with the core proof (500), however old latest.json is" 0 "$(run)"
pub 400 60;   t "behind (400 of 500) but published a minute ago" 0 "$(run)"
pub 400 7200; t "behind (400 of 500) and 2 h old: STALLED" 1 "$(run)"
t "  says so" 1 "$(grep -c 'STALLED: published block 400, the follower should be at 500' "$W/out")"
t "the margin bounds the target (tip 503 - 6 = 497 > 400): still stalled" 1 "$(STUB_TIP=503 run)"
pub 497 7200; t "at tip - margin (497 of tip 503) is caught up" 0 "$(STUB_TIP=503 run)"
pub 400 7200
t "a node RPC failure: could not check" 2 "$(STUB_RPC_FAIL=1 run)"
t "a core-info failure: could not check" 2 "$(STUB_CORE_FAIL=1 run)"
echo '{"height": "x"}' > "$W/d/pub/latest.json"; t "an unreadable latest.json: could not check" 2 "$(run)"
pub 500 60
echo "999 500" > "$W/d/forest-heap"; t "the forest heap below FOREST_HEAP_WARN_BYTES" 0 "$(run)"
echo "1001 500" > "$W/d/forest-heap"; t "the forest heap above FOREST_HEAP_WARN_BYTES is failing" 1 "$(run)"
t "  says so" 1 "$(grep -c 'FOREST HEAP: 1001 bytes at block 500' "$W/out")"
[ "$fails" -eq 0 ] && echo UTREEXO_CHECK_OK || { echo "UTREEXO_CHECK_FAILED ($fails)"; exit 1; }
