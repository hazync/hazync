#!/bin/bash
# FINAL IMAGE re-run (keys-ch: node format 2 + C heap; metering keys-chb), re-exported by the format-2 host.
# FEASIBILITY Q8: scripts/libzync/ci/blocks.sh's edge blocks on the engine (after tip969.sh). For each: block JSON from
# the local node, hints + engine-export (the product host of the engine branch), every chunk and the trie EXECUTED on
# the engine (keys-ch) against RISC Zero's journals, and the aggregate on a metering build of the same guest
# (bench-noverify: env::verify accepts without proofs) against RISC Zero's aggregate journal, byte by byte.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$PATH
: # final image
E=/root/engine-kit/engine/host/target/release/hzengine; H=/root/engine-src/prover/target/release/host
cd /root/engine || exit 1
if [ ! -f keys-chb/guest.vmexe ]; then
  rm -rf keys-chb && cp -r keys-ch keys-chb && rm -f keys-chb/guest.vmexe keys-chb/image.json keys-chb/baseline.json
  HZ_GUEST_FEATURES=bench-noverify HZ_BASE=/root/engine-base6 taskset -c 0-15 $E build --dir keys-chb \
    --guest /root/engine-guestch/prover/methods/guest > edgef_b_build.log 2>&1 || { echo B26_BUILD_FAIL; exit 1; }
  echo "metering image: $(grep -a '^build:' edgef_b_build.log | cut -c1-110)"
fi
mkdir -p edgef
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  J=/root/engine-src/prover/testdata/boundaries/block_$h.json; [ -f $J ] || J=/root/engine-src/prover/block_$h.json; X=/root/engine/edgef/$h
  [ -f $J ] || HZ_TRACKED=/root/engine-src/reproduce/libzync-tracked.json python3 /root/fresh-bench/mkblock.py $h $J > edgef/mk$h.log 2>&1 \
    || { echo "$h MKBLOCK_FAIL"; continue; }
  rm -rf $X
  export HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000 \
         HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_HINT_THREADS=16
  (cd /root/engine-src/prover && $H libzync-hints /root/engine/edgef/$h.lzb > /root/engine/edgef/h$h.log 2>&1 \
     && HAZYNC_LZ_HINTS=/root/engine/edgef/$h.lzb $H engine-export $X > /root/engine/edgef/x$h.log 2>&1) \
    || { echo "$h EXPORT_FAIL: $(grep -a 'panicked\|INVALID' edgef/x$h.log edge/h$h.log | head -1 | cut -c1-120)"; continue; }
  for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
  ok=0; cnt=0
  for f in $X/chunk_*.in $X/trie.in; do [ -f $f ] || continue; b=${f%.in}; cnt=$((cnt+1))
    taskset -c 0-15 $E execute --dir keys-ch --input $f --journal $b.j > $b.exec.log 2>&1 && cmp -s $b.j $b.r0journal && ok=$((ok+1)); done
  ag=$(taskset -c 0-15 $E execute --dir keys-chb --input $X/agg.in --self-id --journal $X/agg.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:\|rror\|panicked\|INVALID" | head -1 | cut -c1-70)
  d=$(cmp -l $X/agg.j $X/agg.r0journal 2>/dev/null | wc -l); la=$(stat -c %s $X/agg.j 2>/dev/null); lr=$(stat -c %s $X/agg.r0journal 2>/dev/null)
  echo "block $h: chunks $(cat $X/chunks 2>/dev/null), chunk+trie journals $ok/$cnt identical; aggregate $la vs RISC Zero $lr bytes, $d bytes differ :: $ag"
done
echo EDGEF_DONE
