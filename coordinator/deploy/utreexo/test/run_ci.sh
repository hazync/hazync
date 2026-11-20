#!/bin/bash
# The follower's fast tests as CI runs them (adversarial.yml, job utreexo-follower): no proofs, no mainnet block files.
#   1. test_check.sh and its control (the check without its stall comparison must miss a stall)
#   2. test_follow_logic.sh on header-only block files made from headers_1_500.txt (mainnet 1..500)
#   3. CONTROLS: the same logic test against copies of the follower with one guard each taken out. Each must FAIL, or
#      the logic test could not tell that guard from its absence. Run in parallel (each its own temporary directory).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
bad=0
bash "$HERE/test_check.sh" > "$W/check.log" 2>&1 || { cat "$W/check.log"; echo "::error::test_check.sh failed"; bad=1; }
bash "$HERE/test_check.sh" --control > "$W/check_ctl.log" 2>&1 || { cat "$W/check_ctl.log"; echo "::error::test_check.sh --control failed"; bad=1; }
python3 -I "$HERE/mkblocks.py" "$HERE/headers_1_500.txt" "$W/blocks" || { echo "::error::mkblocks.py failed"; exit 1; }

# control name, the text taken out of the follower, what replaces it
python3 - "$HERE/../hazync-utreexo-follow" "$W" <<'PY' || { echo "::error::a control's text is no longer in the follower"; exit 1; }
import sys, os
src, w = sys.argv[1], sys.argv[2]
s = open(src).read()
controls = {
    # the fall-back moves EVERY forest aside (the old behaviour): the step cannot start from the kept forest
    'fallback': ('if [ "$f" = "$D/forest.bin" ] || [ "$h" -gt "$NP" ]; then mv "$f" "$O/" || die "mv $f"; fi', 'mv "$f" "$O/" || die "mv $f"'),
    # retention keeps only the newest forest
    'keep': ('[ "$k" -le "$FOREST_KEEP" ] || rm -f "${D:?}/$f"', '[ "$k" -le 1 ] || rm -f "${D:?}/$f"'),
    # work/forest.next is never adopted (F1 a)
    'adopt': ('if [ "$(forest_head "$D/work/forest.next")" = "$P $PX" ] && [ ! -f "$D/forest-$P-$PX.bin" ]; then', 'if false; then'),
    # only a forest at exactly P is used (F1 b)
    'below': ('if [ "$fh" -lt "$P" ] && [ "$(node_hash "$fh")" = "$fx" ]; then FC=$D/$f; FH=$fh; break; fi', ':'),
    # no genesis-replay threshold (F1)
    'genesis': ('if [ "$P" -gt "$GENESIS_REPLAY_MAX" ] && [ "$ALLOW_GENESIS_REPLAY" != 1 ]; then', 'if false; then'),
    # --set-base deletes the old forests (F1 c)
    'setbase': ('if [ -f "$f" ]; then mv "$f" "$O/" || die "mv $f"; fi\n  done\n  rm -f "$D/published"', 'rm -f "$f"\n  done\n  rm -f "$D/published"'),
}
for name, (a, b) in controls.items():
    assert s.count(a) == 1, (name, s.count(a))
    p = os.path.join(w, 'follow_' + name)
    open(p, 'w').write(s.replace(a, b))
    os.chmod(p, 0o755)
print(' '.join(controls))
PY
FOLLOW="$HERE/../hazync-utreexo-follow" bash "$HERE/test_follow_logic.sh" "$W/blocks" > "$W/logic.log" 2>&1 &
pids=("$!")
for c in fallback keep adopt below genesis setbase; do
  FOLLOW="$W/follow_$c" bash "$HERE/test_follow_logic.sh" "$W/blocks" > "$W/ctl_$c.log" 2>&1 &
  pids+=("$!")
done
wait "${pids[0]}"; rc=$?
[ "$rc" = 0 ] && grep -q UTREEXO_FOLLOW_LOGIC_OK "$W/logic.log" || { cat "$W/logic.log"; echo "::error::test_follow_logic.sh failed"; bad=1; }
i=1
for c in fallback keep adopt below genesis setbase; do
  wait "${pids[$i]}"; rc=$?; i=$((i + 1))
  n=$(grep -c '^FAIL' "$W/ctl_$c.log")
  if [ "$rc" != 0 ] && [ "$n" -gt 0 ]; then echo "ok   control $c: $n check(s) fail without that guard"
  else echo "::error::control $c: the logic test PASSED with the guard taken out"; bad=1; fi
done
grep -c '^ok' "$W/logic.log" | sed 's/^/logic checks passed: /'
[ "$bad" = 0 ] && echo UTREEXO_FOLLOWER_CI_OK || { echo UTREEXO_FOLLOWER_CI_FAILED; exit 1; }
