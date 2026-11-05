#!/bin/bash
# E2 step 3: the host trie store in a real chain walk (dev mode: every guest EXECUTED and its claim checked, no GPU).
# 1. mainnet 1..200 from genesis (first spend at 170): per block the store serves mode 17's witness, the aggregate verifies that receipt,
#    and the store's new root must equal the committed trie root (trie_commit asserts it).
# 2. resume: run 1 cut back to 120, HAZYNC_RESUME=1 to 200 loads state_120.trie; its root at 200 must equal run 1's.
# 3. a snapshot with one byte changed, and one swapped for another height's: both refused on resume.
# 4. negative controls on 962,000 (measurement path): list / root / none / counts, each refused.
# usage: test_store.sh <host> <out dir>
H=${1:?host}; O=${2:?out dir}; J=~/libzync-gpu/ssweep; Z=~/dev/hazync-libzync
mkdir -p $O; rm -rf $O/a $O/b
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_SAVE_EACH=1
root() { python3 -c "import json;print(json.load(open('$1'))['ss_hints'])" 2>/dev/null; }
echo "== 1. genesis..200"
$H libzync-prove-range 1 200 $J genesis $O/a > $O/a.log 2>&1; echo "exit=$?"
grep -a 'LZ_PROVE_RANGE_DONE\|panicked\|REJECT' $O/a.log | grep -v 'main.rs:2297' | head -5
echo "trie proofs: $(grep -ac 'trie update proved' $O/a.log)  with marks: $(grep -a 'trie update proved' $O/a.log | grep -vc ': 0 marks')"
echo "root at 200: $(root $O/a/state_200.json)"
cut() { rm -rf $2; cp -r $O/a $2; for k in $(seq $(($1 + 1)) 200); do rm -f $2/block_$k.rcpt $2/state_$k.*; done; }
echo "== 2. resume at 120 (the store reloaded from state_120.trie)"
cut 120 $O/b
HAZYNC_RESUME=1 $H libzync-prove-range 1 200 $J genesis $O/b > $O/b.log 2>&1; echo "exit=$?"
grep -a 'resuming after\|LZ_PROVE_RANGE_DONE' $O/b.log | head -3
echo "same root at 200: $( [ -n "$(root $O/b/state_200.json)" ] && [ "$(root $O/a/state_200.json)" = "$(root $O/b/state_200.json)" ] && echo yes || echo NO)"
echo "== 3. tampered snapshots (resume must refuse)"
for t in byte swap; do
  cut 120 $O/c
  if [ $t = byte ]; then python3 -c "p='$O/c/state_120.trie';b=bytearray(open(p,'rb').read());b[100]^=1;open(p,'wb').write(b)"
  else cp $O/a/state_119.trie $O/c/state_120.trie; fi
  HAZYNC_RESUME=1 $H libzync-prove-range 1 121 $J genesis $O/c > $O/c_$t.log 2>&1; echo "$t: exit=$? $(grep -a 'panicked' -A1 $O/c_$t.log | grep -v 'main.rs:2297\|^--' | grep -av panicked | head -1)"
done
echo "== 4. negative controls, 962,000"
unset RISC0_DEV_MODE HAZYNC_COINLOG_SAVE_EACH; export HAZYNC_CHUNKS=16 HAZYNC_HINT_THREADS=2 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
B=$Z/prover/block_962000.json
HAZYNC_BLOCK=$B $H libzync-hints $O/n.lzb > /dev/null 2>&1
for n in "" list root none counts; do
  HAZYNC_TEST_TRIE=$n HAZYNC_BLOCK=$B HAZYNC_LZ_HINTS=$O/n.lzb $H agg-exec-fake > $O/neg_$n.log 2>&1
  echo "neg=[$n]: $(grep -ao 'REJECT.*\|block validation cycles *[0-9]*\|counts mode' $O/neg_$n.log | head -1)"
done
echo TRIE_STORE_TEST_DONE
