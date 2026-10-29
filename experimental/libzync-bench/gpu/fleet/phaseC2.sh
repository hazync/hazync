#!/bin/bash
# Phase C2: rest of the CORE baseline: chunks 2..15, then the CORE aggregate from the 16 real receipts.
B=/workspace/core/hazync-host-x86_64-linux-gnu-cuda; O=/workspace/outC; cd /workspace/core
export HAZYNC_BLOCK=/workspace/libzync-bundle/block_962000.json HAZYNC_CHUNKS=16 HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost
for i in $(seq 2 15); do
  for ATT in 1 2 3; do
    A0=$SECONDS
    ( HAZYNC_OUT=$O/receipts/chunk_$i.hzk $B prove-chunk $i ) > $O/prove_$i.log 2>&1; RC=$?
    echo "REAL_EXIT=$RC attempt=$ATT" >> $O/prove_$i.log
    [ $RC -eq 0 ] && break
    grep -q "verification indicates proof is invalid" $O/prove_$i.log || break
  done
  echo -e "$i\t$RC\t$((SECONDS-A0))" | tee -a $O/chunks.tsv
  [ $RC -eq 0 ] || { echo "CHUNK_FAIL $i"; exit 1; }
done
A0=$SECONDS
( HAZYNC_RECEIPTS=$O/receipts HAZYNC_AGG_OUT=$O/block_962000.receipt $B agg-chunks ) > $O/agg_prove.log 2>&1; RC=$?
echo "REAL_EXIT=$RC" >> $O/agg_prove.log
echo -e "agg\t$RC\t$((SECONDS-A0))" | tee -a $O/chunks.tsv
echo PHASEC2_DONE
