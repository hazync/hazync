#!/bin/bash
# FEASIBILITY Q8: scripts/libzync/ci/blocks.sh's edge blocks on the engine (after tip969.sh). For each: block JSON from
# the local node, hints + engine-export (the product host of the engine branch), every chunk and the trie EXECUTED on
# the engine (keys-m26) against RISC Zero's journals, and the aggregate on a metering build of the same guest
# (bench-noverify: env::verify accepts without proofs) against RISC Zero's aggregate journal, byte by byte.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
until grep -q "TIP969_DONE\|MKBLOCK_FAIL" /root/engine/tip969.out 2>/dev/null; do sleep 30; done
E=/root/engine-kit/engine/host/target/release/hzengine; H=/root/engine-src/prover/target/release/host
cd /root/engine || exit 1
if [ ! -f keys-b26/guest.vmexe ]; then
  rm -rf keys-b26 && cp -r keys-m26 keys-b26 && rm -f keys-b26/guest.vmexe keys-b26/image.json keys-b26/baseline.json
  HZ_GUEST_FEATURES=bench-noverify HZ_BASE=/root/engine-base5 taskset -c 0-15 $E build --dir keys-b26 \
    --guest /root/engine-guest26/prover/methods/guest > edge_b26_build.log 2>&1 || { echo B26_BUILD_FAIL; exit 1; }
  echo "metering image: $(grep -a '^build:' edge_b26_build.log | cut -c1-110)"
fi
mkdir -p edge
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  J=/root/engine-src/prover/block_$h.json; X=/root/engine/edge/$h
  [ -f $J ] || HZ_TRACKED=/root/engine-src/reproduce/libzync-tracked.json python3 /root/fresh-bench/mkblock.py $h $J > edge/mk$h.log 2>&1 \
    || { echo "$h MKBLOCK_FAIL"; continue; }
  rm -rf $X
  export HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000 \
         HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_HINT_THREADS=16
  (cd /root/engine-src/prover && $H libzync-hints /root/engine/edge/$h.lzb > /root/engine/edge/h$h.log 2>&1 \
     && HAZYNC_LZ_HINTS=/root/engine/edge/$h.lzb $H engine-export $X > /root/engine/edge/x$h.log 2>&1) \
    || { echo "$h EXPORT_FAIL: $(grep -a 'panicked\|INVALID' edge/x$h.log edge/h$h.log | head -1 | cut -c1-120)"; continue; }
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  ok=0; cnt=0
  for f in $X/chunk_*.in $X/trie.in; do [ -f $f ] || continue; b=${f%.in}; cnt=$((cnt+1))
    taskset -c 0-15 $E execute --dir keys-m26 --input $f --journal $b.j > $b.exec.log 2>&1 && cmp -s $b.j $b.r0journal && ok=$((ok+1)); done
  ag=$(taskset -c 0-15 $E execute --dir keys-b26 --input $X/agg.in --self-id --journal $X/agg.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:\|rror\|panicked\|INVALID" | head -1 | cut -c1-70)
  d=$(cmp -l $X/agg.j $X/agg.r0journal 2>/dev/null | wc -l); la=$(stat -c %s $X/agg.j 2>/dev/null); lr=$(stat -c %s $X/agg.r0journal 2>/dev/null)
  echo "block $h: chunks $(cat $X/chunks 2>/dev/null), chunk+trie journals $ok/$cnt identical; aggregate $la vs RISC Zero $lr bytes, $d bytes differ :: $ag"
done
echo EDGE_DONE
