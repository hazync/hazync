#!/bin/bash
# The follower's fast tests as CI runs them (adversarial.yml, job utreexo-follower): no proofs, no mainnet block files.
#   1. test_check.sh and its two controls (the check without its stall comparison must miss a stall; measuring from the
#      last publish alone must page on the first advance after an idle spine)
#   2. test_follow_logic.sh on header-only block files made from headers_1_500.txt (mainnet 1..500)
#   3. CONTROLS: the same logic test against copies of the follower with one guard each taken out, FAIL_FAST=1. Each must
#      fail, and its FIRST failing check must be the one written for that guard (so a control that fails for an unrelated
#      reason does not count). Run in parallel, each in its own temporary directory.
# Measured 20.11.2026 on a laptop (16 threads): see the job's comment in adversarial.yml.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
bad=0
bash "$HERE/test_check.sh" > "$W/check.log" 2>&1 || { cat "$W/check.log"; echo "::error::test_check.sh failed"; bad=1; }
for c in --control --control-core-age; do
  bash "$HERE/test_check.sh" "$c" > "$W/check_ctl.log" 2>&1 || { cat "$W/check_ctl.log"; echo "::error::test_check.sh $c failed"; bad=1; }
done
python3 -I "$HERE/mkblocks.py" "$HERE/headers_1_500.txt" "$W/blocks" || { echo "::error::mkblocks.py failed"; exit 1; }

# name: the text taken out of the follower, its replacement, and the label of the check that must fail first
python3 - "$HERE/../hazync-utreexo-follow" "$W" <<'PY' > "$W/controls" || { echo "::error::a control's text is no longer in the follower"; exit 1; }
import sys, os
src, w = sys.argv[1], sys.argv[2]
s = open(src).read()
controls = {
    # the fall-back moves EVERY forest aside (the old behaviour)
    'fallback': ('if [ "$f" = "$D/forest.bin" ] || [ "$h" -gt "$NP" ]; then mv "$f" "$O/" || die "mv $f"; fi',
                 'mv "$f" "$O/" || die "mv $f"', "the orphaned branch's forest cache (100) moved aside"),
    # retention keeps only the newest forest
    'keep': ('[ "$k" -le "$FOREST_KEEP" ] || rm -f "${D:?}/$f"', '[ "$k" -le 1 ] || rm -f "${D:?}/$f"',
             'the FOREST_KEEP (3) newest forest caches are kept'),
    # work/forest.next is never adopted (F1 a)
    'adopt': ('if [ "$(forest_head "$D/work/forest.next")" = "$P $PX" ] && [ ! -f "$D/forest-$P-$PX.bin" ]; then',
              'if false; then', 'work/forest.next adopted as the forest of 9'),
    # only a forest at exactly P is used (F1 b)
    'below': ('      if [ "$fh" -lt "$P" ]; then\n', '      if false; then\n', 'started from the newest kept forest below it (80)'),
    # no genesis-replay threshold (F1)
    'genesis': ('if [ "$P" -gt "$GENESIS_REPLAY_MAX" ] && [ "$ALLOW_GENESIS_REPLAY" != 1 ]; then', 'if false; then',
                'a genesis replay above GENESIS_REPLAY_MAX is refused'),
    # --set-base deletes the old forests (F1 c)
    'setbase': ('if [ -f "$f" ]; then mv "$f" "$O/" || die "mv $f"; fi\n  done\n  rm -f "$D/published"',
                'rm -f "$f"\n  done\n  rm -f "$D/published"', "the old base's forests moved aside, not deleted"),
    # an unusable forest is not moved aside and retried (round 2 #2)
    'retry': ('if [ "$rc" = 4 ] && [ "${#req[@]}" -gt 0 ]; then', 'if false; then', 'an unusable forest at P (round 2 #2)'),
    # forests above H take retention slots (round 2 #5)
    'above': ('if ! [[ "$fh" =~ ^[0-9]+$ ]] || [ "$fh" -gt "$H" ]; then', 'if false; then',
              'the forest above H moved aside, the two newest at or below H kept'),
    # an RPC failure while choosing the forest reads as "another branch" (round 2 #4)
    'nodehash': ('nx=$(node_hash "$fh") || die "the node did not answer getblockhash $fh"', 'nx=$(node_hash "$fh") || nx=',
                 'an RPC failure while choosing the forest is an error (round 2 #4)'),
    # DRY_RUN adopts work/forest.next (round 2 #8)
    'dryrun': ('if [ "${DRY_RUN:-0}" != 1 ] && [ -f "$D/work/forest.next" ]; then', 'if [ -f "$D/work/forest.next" ]; then',
               'DRY_RUN=1 adopts nothing and prunes nothing (round 2 #8)'),
}
for name, (a, b, label) in controls.items():
    assert s.count(a) == 1, (name, s.count(a))
    p = os.path.join(w, 'follow_' + name)
    open(p, 'w').write(s.replace(a, b))
    os.chmod(p, 0o755)
    print(name + '\t' + label)
PY
FOLLOW="$HERE/../hazync-utreexo-follow" bash "$HERE/test_follow_logic.sh" "$W/blocks" > "$W/logic.log" 2>&1 &
pids=("$!"); names=()
while IFS=$'\t' read -r c _; do
  FAIL_FAST=1 FOLLOW="$W/follow_$c" bash "$HERE/test_follow_logic.sh" "$W/blocks" > "$W/ctl_$c.log" 2>&1 &
  pids+=("$!"); names+=("$c")
done < "$W/controls"
wait "${pids[0]}"; rc=$?
[ "$rc" = 0 ] && grep -q UTREEXO_FOLLOW_LOGIC_OK "$W/logic.log" || { cat "$W/logic.log"; echo "::error::test_follow_logic.sh failed"; bad=1; }
i=1
while IFS=$'\t' read -r c label; do
  wait "${pids[$i]}"; rc=$?; i=$((i + 1))
  first=$(grep -m1 '^FAIL' "$W/ctl_$c.log")
  if [ "$rc" != 0 ] && [[ "$first" == *"$label"* ]]; then echo "ok   control $c: fails first at '$label'"
  else echo "::error::control $c: want the first failure at '$label', got '${first:-none}' (exit $rc)"; bad=1; fi
done < "$W/controls"
[ "${#names[@]}" -gt 0 ] || { echo "::error::no controls ran"; bad=1; }
grep -c '^ok' "$W/logic.log" | sed 's/^/logic checks passed: /'
[ "$bad" = 0 ] && echo UTREEXO_FOLLOWER_CI_OK || { echo UTREEXO_FOLLOWER_CI_FAILED; exit 1; }
