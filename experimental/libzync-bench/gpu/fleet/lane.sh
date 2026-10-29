#!/bin/bash
# lane.sh "<chunk indices>": prove those chunks on this pod's GPU (#119 retry). Epoch start/end recorded.
. /workspace/lzenv.sh; [ -f /workspace/blockenv.sh ] && . /workspace/blockenv.sh; cd /workspace/hazync/prover; O=/workspace/lane; mkdir -p $O/receipts
export HAZYNC_CHUNKS=16 HAZYNC_BATCH_HINTS_DIR=${HG:-/workspace/libzync-bundle/hg}
echo "START $(date +%s.%N)" > $O/lane.tsv
for i in $1; do
  for ATT in 1 2 3 4; do
    A0=$(date +%s.%N)
    HAZYNC_OUT=$O/receipts/chunk_$i.hzk target/release/host prove-chunk $i > $O/prove_$i.log 2>&1; RC=$?
    echo "REAL_EXIT=$RC attempt=$ATT" >> $O/prove_$i.log
    [ $RC -eq 0 ] && break
    grep -q "verification indicates proof is invalid" $O/prove_$i.log || break
  done
  echo -e "$i\t$RC\t$A0\t$(date +%s.%N)" >> $O/lane.tsv
  [ $RC -eq 0 ] || { echo LANE_FAIL >> $O/lane.tsv; exit 1; }
done
echo "END $(date +%s.%N)" >> $O/lane.tsv; echo LANE_DONE >> $O/lane.tsv
