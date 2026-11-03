#!/bin/bash
# E1 sharding measurement: for each block in the variant cost table, the outpoint tree's removes / inserts (from the
# c10o run's "itree 0:" line) split into 1, 16 and 64 key-prefix shards; per-shard cycles in execute mode.
# 1 shard = calibration against c10o's aggregate growth (c10o aggregate - c10b aggregate).
# usage: shard_bench.sh <cost dir> <zyncapp binary>
C=${1:?cost dir}; Z=${2:?zyncapp binary}; N=${HAZYNC_ITREE_N:-180000000}
for l in $C/*_c10o.log; do
  n=$(basename $l _c10o.log)
  read r i < <(grep -a 'itree 0:' $l | awk '{print $3, $5}')
  [ -z "$r" ] && continue
  for s in 1 16 64; do
    $Z bench-shard $r $i $N $s > $C/${n}_shard$s.log 2>&1
    echo "$n $(grep BENCH_SHARD $C/${n}_shard$s.log || echo FAILED)"
  done
done
echo SHARD_BENCH_DONE
