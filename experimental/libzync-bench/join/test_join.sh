#!/bin/bash
# ROADMAP H-B: blocks proved in any order on the host's states, joined in a tree, must prove EXACTLY what the linear
# chain proves (dev mode: every guest executed and its claim checked, no GPU; the real recursion is checked in the GPU
# benchmark step). Exits non-zero on any miss; prints JOIN_TEST_PASS only if every check holds.
# 1. leaves: blocks 1..N, each a base step on the linear run's state before it (block 1 on genesis), LAST FIRST.
# 2. a binary join tree over the leaves (guest mode 18), bottom-up; N=200 has odd carries (25 -> 13 -> 7).
# 3. the root's journal must be byte-identical to the linear run's block_N journal.
# 4. mixed shapes: join(linear block_100, leaf 101) must be byte-identical to linear block_101.
# 5. negatives, each refused with the GUEST's message (not just any failure): a gap, swapped order, the same block
#    twice, a leaf on a tampered anchor (max_time: not cross-checked by the host, so it must reach the join); and a
#    receipt of another image, refused by the host before proving.
# usage: test_join.sh <host> <linear run dir with every state (test_store.sh's a/)> <out dir> [N] [other-image receipt]
# P=<jobs> (default 3): leaves, and each level's joins, run in parallel (they are independent: the point of H-B).
H=${1:?host}; A=${2:?linear run dir}; O=${3:?out dir}; N=${4:-200}; OTHER=${5:-}; J=~/libzync-gpu/ssweep
mkdir -p $O; rm -rf $O/leaf $O/join $O/badleaf* $O/neg_*; mkdir -p $O/leaf $O/join
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_SAVE_EACH=1
fails=0; miss() { echo "FAIL: $*"; fails=$((fails + 1)); }
sha() { $H journal-sha $1 2>/dev/null | awk '/JOURNAL_SHA256/{print $2}'; }
leaf() {  # leaf <h> <anchor json> <out dir>
  rm -rf $3; mkdir -p $3
  $H libzync-prove-range $1 $1 $J $2 $3 > $3.log 2>&1 && [ -f $3/block_$1.rcpt ]
}
P=${P:-3}
export H A J O
export -f leaf
echo "== 1. $N leaves, last block first, $P at a time"
seq $N -1 1 | xargs -P $P -I{} bash -c 'h={}; if [ $h = 1 ]; then a=genesis; else a=$A/state_$((h - 1)).json; fi
  leaf $h $a $O/leaf/$h || echo "LEAF $h FAILED: $(grep -a panicked -A1 $O/leaf/$h.log | head -2 | tr "\n" " ")"' | tee $O/leaf_fail.txt
[ -s $O/leaf_fail.txt ] && exit 1
echo "leaves: $(ls $O/leaf/*/block_*.rcpt | wc -l)"
echo "== 2. join tree"
lvl=0; cur=(); for h in $(seq 1 $N); do cur+=($O/leaf/$h/block_$h.rcpt); done
joins=0
while [ ${#cur[@]} -gt 1 ]; do
  lvl=$((lvl + 1)); nxt=(); pairs=()
  for ((i = 0; i < ${#cur[@]}; i += 2)); do
    if [ $((i + 1)) -lt ${#cur[@]} ]; then
      o=$O/join/l${lvl}_$i.rcpt
      pairs+=("${cur[$i]} ${cur[$((i + 1))]} $o")
      joins=$((joins + 1)); nxt+=($o)
    else nxt+=(${cur[$i]}); fi
  done
  printf '%s\n' "${pairs[@]}" | xargs -P $P -L1 bash -c '$H libzync-join $0 $1 $2 > $2.log 2>&1 || echo "JOIN FAILED $2: $(grep -a panicked -A1 $2.log | head -2 | tr "\n" " ")"' > $O/join_fail.txt
  [ -s $O/join_fail.txt ] && { cat $O/join_fail.txt; exit 1; }
  echo "level $lvl: ${#nxt[@]} proofs"
  cur=("${nxt[@]}")
done
[ $joins = $((N - 1)) ] && echo "joins: $joins (= N-1)" || miss "joins $joins != N-1"
echo "== 3. root == linear chain"
R=$(sha ${cur[0]}); L=$(sha $A/block_$N.rcpt)
echo "root   $R"; echo "linear $L"
[ -n "$R" ] && [ "$R" = "$L" ] && echo "IDENTICAL: yes" || miss "root journal != linear block_$N"
echo "== 4. mixed: join(linear block_100, leaf 101) == linear block_101"
if [ $N -ge 101 ]; then
  $H libzync-join $A/block_100.rcpt $O/leaf/101/block_101.rcpt $O/join/mixed.rcpt > $O/join/mixed.log 2>&1 || miss "mixed join failed"
  M=$(sha $O/join/mixed.rcpt); L1=$(sha $A/block_101.rcpt)
  [ -n "$M" ] && [ "$M" = "$L1" ] && echo "IDENTICAL: yes" || miss "mixed join journal != linear block_101"
fi
echo "== 5. negatives (each must be refused with the guest's own message)"
LINK="join: the right proof does not start where the left one ends"
neg() {  # neg <name> <left> <right> <expected message>
  if $H libzync-join $2 $3 $O/neg_$1.rcpt > $O/neg_$1.log 2>&1; then miss "$1: ACCEPTED"
  elif grep -aqF "$4" $O/neg_$1.log; then echo "$1: refused ($4)"
  else miss "$1: refused, but not with '$4': $(grep -a panicked -A1 $O/neg_$1.log | grep -av panicked | head -1)"; fi
}
neg gap $O/leaf/1/block_1.rcpt $O/leaf/3/block_3.rcpt "$LINK"
neg swapped $O/leaf/2/block_2.rcpt $O/leaf/1/block_1.rcpt "$LINK"
neg twice $O/leaf/1/block_1.rcpt $O/leaf/1/block_1.rcpt "$LINK"
python3 - $A/state_1.json $O/bad_anchor.json <<'EOF'
import json, sys
s = json.load(open(sys.argv[1]))
s["max_time"] += 1   # not cross-checked by the host: only the join can catch it
json.dump(s, open(sys.argv[2], "w"))
EOF
for x in coinlog trie; do cp $A/state_1.$x $O/bad_anchor.$x; done
if leaf 2 $O/bad_anchor.json $O/badleaf; then neg tampered_anchor $O/leaf/1/block_1.rcpt $O/badleaf/block_2.rcpt "$LINK"
else miss "tampered_anchor: the host refused the leaf, so the join was never exercised: $(grep -a panicked -A1 $O/badleaf.log | grep -av panicked | head -1)"; fi
if [ -n "$OTHER" ]; then neg other_image $O/leaf/1/block_1.rcpt $OTHER "does not verify against this image"; fi
[ $fails = 0 ] && echo JOIN_TEST_PASS || { echo "JOIN_TEST_FAILED ($fails)"; exit 1; }
