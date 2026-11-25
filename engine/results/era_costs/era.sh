#!/bin/bash
# FEASIBILITY Q16: engine cost across eras. For each height: block JSON from the local node, hints + engine-export (16
# chunks max, cost-sized), then every chunk + trie metered on the engine (hz24 keys) and the aggregate on the metering
# image (b24, bench-noverify). Prints the per-block cost sums.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
while ! grep -q "CV_DONE" /root/engine/cv.out; do sleep 30; done
E=/root/engine-kit/engine/host/target/release/hzengine; H=/root/engine-src/prover/target/release/host
for h in 100000 200000 250000 300000 400000 500000 600000 700000 800000 900000; do
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  J=/root/engine-src/prover/block_$h.json; X=/root/engine/era/$h; rm -rf $X; mkdir -p /root/engine/era
  [ -f $J ] || HZ_TRACKED=/root/engine-src/reproduce/libzync-tracked.json python3 /root/fresh-bench/mkblock.py $h $J > /root/engine/era/mk$h.log 2>&1 || { echo "$h MKBLOCK_FAIL"; continue; }
  export HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000 HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_HINT_THREADS=16
  (cd /root/engine-src/prover && $H libzync-hints /root/engine/era/$h.lzb > /root/engine/era/h$h.log 2>&1 && HAZYNC_LZ_HINTS=/root/engine/era/$h.lzb $H engine-export $X > /root/engine/era/x$h.log 2>&1) || { echo "$h EXPORT_FAIL"; continue; }
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  n=$(cat $X/chunks); tot=0; ok=0; cnt=0
  for f in $X/chunk_*.in $X/trie.in; do b=${f%.in}; cnt=$((cnt+1))
    c=$($E execute --dir /root/engine/keys-hz24 --input $f --journal $b.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:" | sed -E "s/.*cost ([0-9]+).*/\1/")
    cmp -s $b.j $b.r0journal && ok=$((ok+1)); tot=$((tot+${c:-0})); done
  a=$($E execute --dir /root/engine/keys-b24 --input $X/agg.in --self-id 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:" | sed -E "s/.*cost ([0-9]+).*/\1/")
  ins=$(grep -ao "([0-9]* inputs" /root/engine/era/x$h.log | tr -dc 0-9)
  echo "block $h: inputs ${ins:-?}, chunks $n, journals $ok/$cnt identical, chunk+trie cost $tot, aggregate cost ${a:-?}"
done
echo ERA_DONE
