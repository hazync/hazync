#!/bin/bash
# C8 (C) again, the RISC Zero side under the engine's chunking (HAZYNC_CHUNKS=16, HAZYNC_CHUNK_TARGET=900000000: what
# the host sets itself when it runs on the engine or exports, and what era.sh set for libzync-hints). c.sh ran the RISC
# Zero hints with the RISC Zero default chunking, so its bundles had another layout (block 130,000: 2 chunks vs 1).
set -uo pipefail
for v in $(compgen -e | grep "^HAZYNC_\|^HZ_"); do unset "$v"; done
W=/root/c8w; K=$W/kit; X=$W/cx; t0=$(date +%s); say(){ echo "[$(( $(date +%s)-t0 )) s] $*"; }
until grep -q C_DONE $W/c.out 2>/dev/null; do sleep 60; done
for h in 130000 140000 741000 962000 965500; do
  [ -d $X/e_$h ] || { say "$h: no engine export (c.sh failed there)"; continue; }
  export HAZYNC_BLOCK=$K/prover/block_$h.json HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=2 HAZYNC_COINLOG_TXCOUNTS=$K/experimental/libzync-bench/cost/txcounts.txt HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000
  rm -rf $X/r1_$h
  ( cd $K/prover && taskset -c 7 nice $W/host_r0 libzync-hints $X/$h.r1.lzb > $X/h$h.r1.log 2>&1 && HAZYNC_LZ_HINTS=$X/$h.r1.lzb taskset -c 7 nice $W/host_r0 engine-export $X/r1_$h > $X/x$h.r1.log 2>&1 ) || { say "$h R0_FAIL"; tail -3 $X/x$h.r1.log; continue; }
  unset HAZYNC_BLOCK
  r=$(cmp -s $X/$h.r1.lzb $X/$h.e.lzb && echo "bundle same" || echo "bundle DIFFERS")
  for f in $(cd $X/r1_$h && ls | grep -v "cycles$\|method_id$\|^engine$\|agg.r0journal"); do
    if [ "$f" = agg.in ] || [ "$f" = agg_collect.in ]; then
      python3 -c "import sys;a=open(sys.argv[1],'rb').read();b=open(sys.argv[2],'rb').read();sys.exit(0 if len(a)==len(b) and a[:4]==b[:4] and a[36:]==b[36:] else 1)" $X/r1_$h/$f $X/e_$h/$f && r="$r; $f same (but bytes 4..36, METHOD_ID)" || r="$r; $f DIFFERS"
    else cmp -s $X/r1_$h/$f $X/e_$h/$f && r="$r; $f same" || r="$r; $f DIFFERS"; fi
  done
  say "block $h: $r"
done
say C2_DONE
