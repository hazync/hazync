#!/bin/bash
# work.sh <host:port> <id>: one aggregate worker on this pod's GPU.
. /workspace/lzenv.sh; cd /workspace/hazync/prover; mkdir -p /workspace/agg
HAZYNC_WORKER_ID=$2 target/release/host seg-connect $1 > /workspace/agg/worker_$2.log 2>&1; echo "REAL_EXIT=$?" >> /workspace/agg/worker_$2.log
