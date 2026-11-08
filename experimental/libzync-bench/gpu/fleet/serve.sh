#!/bin/bash
# serve.sh: the aggregate coordinator on pod A, open to the other pods (HAZYNC_SEG_REMOTE=1).
. /workspace/lzenv.sh; [ -f /workspace/blockenv.sh ] && . /workspace/blockenv.sh; cd /workspace/hazync/prover; O=/workspace/agg; mkdir -p $O
export HAZYNC_CHUNKS=16 HAZYNC_RECEIPTS=/workspace/lane/receipts HAZYNC_AGG=1 HAZYNC_SEG_REMOTE=1 HAZYNC_OUT=$O/block_spread.receipt
echo "START $(date +%s.%N)" > $O/times
target/release/host seg-serve > $O/serve.log 2>&1; echo "REAL_EXIT=$?" >> $O/serve.log
echo "END $(date +%s.%N)" >> $O/times; echo SERVE_DONE >> $O/times
