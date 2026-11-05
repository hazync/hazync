#!/bin/bash
# ROADMAP H-B: blocks proved in any order on the host's states, joined in a tree, must prove EXACTLY what the linear
# chain proves (dev mode: every guest executed and its claim checked, no GPU).
# 1. leaves: blocks 1..N, each a base step on the linear run's state before it (block 1 on genesis), in REVERSE order.
# 2. a binary join tree over the leaves (guest mode 18), bottom-up.
# 3. the root's journal must be byte-identical to the linear run's block_N journal (JOURNAL_SHA256).
# 4. negatives, each refused by the guest: a gap, swapped order, the same block twice, a leaf on a tampered anchor.
# usage: test_join.sh <host> <linear run dir with every state (test_store.sh's a/)> <out dir> [N, default 200]
H=${1:?host}; A=${2:?linear run dir}; O=${3:?out dir}; N=${4:-200}; J=~/libzync-gpu/ssweep
mkdir -p $O; rm -rf $O/leaf $O/join; mkdir -p $O/leaf $O/join
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_SAVE_EACH=1
leaf() {  # leaf <h> <anchor json> <out dir>
  rm -rf $3; mkdir -p $3
  $H libzync-prove-range $1 $1 $J $2 $3 > $3.log 2>&1 && [ -f $3/block_$1.rcpt ]
}
echo "== 1. $N leaves, last block first"
for h in $(seq $N -1 1); do
  if [ $h = 1 ]; then a=genesis; else a=$A/state_$((h - 1)).json; fi
  leaf $h $a $O/leaf/$h || { echo "LEAF $h FAILED: $(grep -a panicked -A1 $O/leaf/$h.log | head -2 | tr '\n' ' ')"; exit 1; }
done
echo "leaves: $(ls $O/leaf/*/block_*.rcpt | wc -l)"
echo "== 2. join tree"
lvl=0; cur=(); for h in $(seq 1 $N); do cur+=($O/leaf/$h/block_$h.rcpt); done
joins=0
while [ ${#cur[@]} -gt 1 ]; do
  lvl=$((lvl + 1)); nxt=()
  for ((i = 0; i < ${#cur[@]}; i += 2)); do
    if [ $((i + 1)) -lt ${#cur[@]} ]; then
      o=$O/join/l${lvl}_$i.rcpt
      $H libzync-join ${cur[$i]} ${cur[$((i + 1))]} $o > $o.log 2>&1 || { echo "JOIN FAILED l$lvl $i: $(grep -a panicked -A1 $o.log | head -2 | tr '\n' ' ')"; exit 1; }
      joins=$((joins + 1)); nxt+=($o)
    else nxt+=(${cur[$i]}); fi
  done
  echo "level $lvl: ${#nxt[@]} proofs"
  cur=("${nxt[@]}")
done
echo "joins: $joins (N-1 = $((N - 1)))"
echo "== 3. root == linear chain"
R=$($H journal-sha ${cur[0]} | awk '/JOURNAL_SHA256/{print $2}')
L=$($H journal-sha $A/block_$N.rcpt | awk '/JOURNAL_SHA256/{print $2}')
echo "root   $R"; echo "linear $L"
[ -n "$R" ] && [ "$R" = "$L" ] && echo "IDENTICAL: yes" || echo "IDENTICAL: NO"
echo "== 4. negatives (each must be refused)"
neg() { $H libzync-join $2 $3 $O/neg_$1.rcpt > $O/neg_$1.log 2>&1 && echo "$1: ACCEPTED (BAD)" || echo "$1: refused: $(grep -ao 'join: [^"]*' $O/neg_$1.log | head -1)"; }
neg gap $O/leaf/1/block_1.rcpt $O/leaf/3/block_3.rcpt
neg swapped $O/leaf/2/block_2.rcpt $O/leaf/1/block_1.rcpt
neg twice $O/leaf/1/block_1.rcpt $O/leaf/1/block_1.rcpt
python3 - $A/state_1.json $O/bad_anchor.json <<'EOF'
import json, sys
s = json.load(open(sys.argv[1]))
s["max_time"] += 1   # not cross-checked by the host: only the join can catch it
json.dump(s, open(sys.argv[2], "w"))
EOF
for x in coinlog trie; do cp $A/state_1.$x $O/bad_anchor.$x; done
if leaf 2 $O/bad_anchor.json $O/badleaf; then neg tampered_anchor $O/leaf/1/block_1.rcpt $O/badleaf/block_2.rcpt
else echo "tampered_anchor: leaf refused by the host before a join: $(grep -a panicked -A1 $O/badleaf.log | grep -av panicked | head -1)"; fi
echo JOIN_TEST_DONE
