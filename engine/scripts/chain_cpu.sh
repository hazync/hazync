#!/bin/bash
# chain_cpu.sh <product host binary> <keys dir> <blocks dir> <out dir>: the engine end to end through the PRODUCT host
# (engine_backend), on CPU: blocks 1..2 from genesis (block 2 verifies block 1's proof in-guest), block 3 as a base
# step on S(2), the join of 2 and 3 (both verified in-guest, adjacency checked), and libzync-verify on each.
# Early blocks are coinbase-only (no chunks): trie + aggregate (+ previous block) per block.
set -u
H=$1; K=$2; B=$3; O=$4; mkdir -p $O
export HAZYNC_ENGINE=/root/engine/host/target/release/hzengine HAZYNC_ENGINE_KEYS=$K RAYON_NUM_THREADS=${THREADS:-10}
CPUS=${CPUS:-6-15}
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
t taskset -c $CPUS $H libzync-prove-range 1 2 $B genesis $O/a > $O/a.log 2>&1 || { tail -30 $O/a.log; exit 1; }
grep -aE "engine:|block [0-9]+:|LZ_PROVE_RANGE_DONE" $O/a.log | head -30
t taskset -c $CPUS $H libzync-prove-range 3 3 $B $O/a/state_2.json $O/b > $O/b.log 2>&1 || { tail -30 $O/b.log; exit 1; }
grep -aE "engine:|block [0-9]+:|LZ_PROVE_RANGE_DONE" $O/b.log | head -20
t taskset -c $CPUS $H libzync-join $O/a/block_2.ovm $O/b/block_3.ovm $O/join_1_3.ovm > $O/join.log 2>&1 || { tail -30 $O/join.log; exit 1; }
grep -aE "JOINED|JOURNAL" $O/join.log
for p in a/block_1.ovm a/block_2.ovm join_1_3.ovm; do
  echo "== libzync-verify $p"; $H libzync-verify $O/$p 2>&1 | grep -aE "\[(ok|FAIL)\]|LIBZYNC_VERIFY" | head -30
done
echo "== b/block_3.ovm (base on S(2): anchored, not genesis)"; $H libzync-verify $O/b/block_3.ovm $O/a/state_2.json 2>&1 | grep -aE "\[(ok|FAIL)\]|LIBZYNC_VERIFY" | head -30
echo CHAIN_CPU_DONE
