#!/bin/bash
# tiphint_pod.sh <height>: on a pod, collect batch + ECMH hints for /workspace/block_<h>.json, then a hinted check.
. /workspace/lzenv.sh; H=$1; O=/workspace/tip_$H; mkdir -p $O/hg; cd /workspace/hazync/prover
HOST=target/release/host; HOOK=/workspace/hazync/experimental/libzync-bench/hook
export HAZYNC_BLOCK=/workspace/block_$H.json HAZYNC_CHUNKS=16 HAZYNC_PROFILE_EXEC=1
T0=$SECONDS
HAZYNC_BATCH_HINTS_DIR=$O/hg HAZYNC_ECMH_HINTS=/nonexistent $HOST chunk-profile > $O/collect16.log 2>&1; echo "REAL_EXIT=$?" >> $O/collect16.log
python3 -I $HOOK/make_batch_hints.py $O/collect16.log $O/hg > $O/mk_batch.txt 2>&1
HAZYNC_BATCH_HINTS_DIR=$O/hg HAZYNC_ECMH_HINTS=/nonexistent $HOST agg-exec-fake > $O/ecmh_collect.log 2>&1; echo "REAL_EXIT=$?" >> $O/ecmh_collect.log
python3 -I $HOOK/make_ecmh_hints.py $O/ecmh_collect.log $O/ecmh.bin > $O/mk_ecmh.txt 2>&1
T1=$SECONDS
HAZYNC_BATCH_HINTS_DIR=$O/hg HAZYNC_ECMH_HINTS=$O/ecmh.bin $HOST agg-exec-fake > $O/check16.log 2>&1; echo "REAL_EXIT=$?" >> $O/check16.log
echo "hint collection $((T1-T0)) s, check $((SECONDS-T1)) s" > $O/summary.txt
echo "ok=true x$(grep -a -c 'ok=true' $O/check16.log), $(grep -a -o 'fallback=[0-9]*' $O/check16.log | sort | uniq -c | tr '\n' ' ')" >> $O/summary.txt
grep -a acc_x_top_word $O/check16.log | tail -1 >> $O/summary.txt; tail -1 $O/check16.log >> $O/summary.txt
echo TIPHINT_DONE >> $O/summary.txt
