#!/bin/bash
# ROADMAP H-B 8 (11-07): prep's per-block leaf bundles.
#  1. prep 1..200 writes leaf_<h>.bin, its states checked against the proved c14 run;
#  2. prove-range 1..200 (dev mode) asserts every block's witnesses == its bundle's (HAZYNC_LEAF_CHECK);
#  3. leaf proofs of 198, 199, 200 from their bundles alone (an EMPTY block dir but the block's own file);
#  4. linear 197 + leaf 198 + leaf 199 + leaf 200 joined == the linear block 200's journal.
set -u
# H (host), O (out), J (block files 1..200), REF (a linear run 1..200 with every state and block_*.rcpt): overridable (CI)
R=~/libzync-gpu/r18; H=${H:-$R/host_c14leaf}; O=${O:-~/libzync-gpu/c14b/leaf}; rm -rf $O; mkdir -p $O
J=${J:-$HOME/libzync-gpu/ssweep}; REF=${REF:-$HOME/libzync-gpu/c14/m200}
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
HAZYNC_PREP_LEAF_BUNDLES=$O/leaves HAZYNC_PREP_CHECK=$REF $H libzync-prep 1 200 $J genesis $O/prep > $O/prep.log 2>&1
echo "prep EXIT=$?"; grep -a 'LZ_PREP_DONE\|PREP_MISMATCH\|panicked' $O/prep.log | head -3
echo "bundles: $(ls $O/leaves | grep -c '^leaf_.*\.bin$'), $(du -sh $O/leaves | cut -f1); largest $(ls -S $O/leaves | head -1) $(stat -c %s $O/leaves/$(ls -S $O/leaves | head -1)) B"
# 3 first (minutes), then 2 (the long one)
for h in 198 199 200; do
  mkdir -p $O/b$h; cp $J/block_$h.json $O/b$h/
  HAZYNC_LEAF_BUNDLE=$O/leaves/leaf_$h.bin $H libzync-prove-range $h $h $O/b$h leaf $O/leaf$h > $O/leaf$h.log 2>&1
  echo "leaf $h EXIT=$?"; grep -a 'panicked\|LZ_PROVE_RANGE_DONE' $O/leaf$h.log | head -2
  python3 - "$O/leaf$h/state_$h.json" "$REF/state_$h.json" <<'EOF'
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:])
d = [k for k in a if a[k] != b.get(k)]
print("  leaf state vs linear: differs only in", d, "(want ['anchor_id'])")
EOF
done
$H libzync-join $REF/block_197.rcpt $O/leaf198/block_198.rcpt $O/j198.rcpt > $O/join.log 2>&1; echo "join 197+198 EXIT=$?"
$H libzync-join $O/j198.rcpt $O/leaf199/block_199.rcpt $O/j199.rcpt >> $O/join.log 2>&1; echo "join +199 EXIT=$?"
$H libzync-join $O/j199.rcpt $O/leaf200/block_200.rcpt $O/j200.rcpt >> $O/join.log 2>&1; echo "join +200 EXIT=$?"
grep -a 'JOINED\|panicked' $O/join.log
A=$($H journal-sha $O/j200.rcpt | awk '/^JOURNAL_SHA256/{print $2}'); B=$($H journal-sha $REF/block_200.rcpt | awk '/^JOURNAL_SHA256/{print $2}')
echo "joined 197+198+199+200 journal ${A:0:16} vs linear block 200 ${B:0:16}: $([ -n "$A" ] && [ "$A" = "$B" ] && echo EQUAL || echo DIFFERENT)"
HAZYNC_LEAF_CHECK=$O/leaves $H libzync-prove-range 1 200 $J genesis $O/check > $O/check.log 2>&1
echo "prove-range with HAZYNC_LEAF_CHECK EXIT=$?"; grep -a 'LEAF_CHECK\|panicked\|LZ_PROVE_RANGE_DONE' $O/check.log | head -3
cmp $O/check/state_200.json $REF/state_200.json && echo "check run: state_200 == the c14 pass's"
echo LEAF_TEST_DONE
