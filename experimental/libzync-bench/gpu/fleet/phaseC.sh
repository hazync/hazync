#!/bin/bash
# Phase C: CORE baseline on the same card. Canonical v0.22.1 host (METHOD_ID 37987b85), block 962,000 in
# 16 chunks: execute-profile all 16 for per-chunk cycles, then PROVE chunks 0 and 1 for seconds.
B=/workspace/core/hazync-host-x86_64-linux-gnu-cuda; O=/workspace/outC; mkdir -p $O/receipts; cd /workspace/core
export HAZYNC_BLOCK=/workspace/libzync-bundle/block_962000.json HAZYNC_CHUNKS=16 HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost
( HAZYNC_PROFILE_EXEC=1 $B chunk-profile ) > $O/profile.log 2>&1; echo "REAL_EXIT=$?" >> $O/profile.log
for i in 0 1; do
  ( nvidia-smi --query-gpu=memory.used,power.draw --format=csv,noheader -l 2 > $O/smi_$i.log 2>/dev/null ) & SMI=$!
  for ATT in 1 2 3; do
    A0=$SECONDS
    ( HAZYNC_OUT=$O/receipts/chunk_$i.hzk $B prove-chunk $i ) > $O/prove_$i.log 2>&1; RC=$?
    echo "REAL_EXIT=$RC attempt=$ATT" >> $O/prove_$i.log
    [ $RC -eq 0 ] && break
    grep -q "verification indicates proof is invalid" $O/prove_$i.log || break
  done
  kill $SMI
  echo -e "$i\t$RC\t$((SECONDS-A0))" | tee -a $O/chunks.tsv
done
echo PHASEC_DONE
