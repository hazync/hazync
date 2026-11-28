#!/bin/bash
# C8 (C) on hazync-coord (28.11.2026), inside a private mount namespace: libzync-hints + engine-export on the ENGINE vs
# the same commit's RISC Zero host (its guest built here), on the repo's five real blocks; every exported file compared.
set -uo pipefail
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_\|^HZ_"); do unset "$v"; done
export CARGO_BUILD_JOBS=4
W=/root/c8w; KK=/root/engine-kit; t0=$(date +%s); say(){ echo "[$(( $(date +%s)-t0 )) s] $*"; }
mount --bind $W/kit $KK || { echo MOUNT_FAIL; exit 1; }
git -C $KK pull -q --ff-only origin engine/openvm; say "kit $(git -C $KK log --oneline -1 | cut -c1-60)"
E=$W/hzengine
(cd $KK/prover && env HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 taskset -c 4-7 nice cargo build --release -p host > $W/host-r0.log 2>&1) \
  && cp $KK/prover/target/release/host $W/host_r0 && say "RISC Zero host built (its guest too)" || { echo HOST_R0_FAIL; grep -a "^error" -A6 $W/host-r0.log | head; exit 1; }
(cd $KK/prover && env RISC0_SKIP_BUILD=1 HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 taskset -c 4-7 nice cargo build --release -p host > $W/host-e.log 2>&1) \
  && cp $KK/prover/target/release/host $W/host_e && say "engine host built (no guest)" || { echo HOST_E_FAIL; exit 1; }
X=$W/cx; rm -rf $X; mkdir -p $X
for h in 130000 140000 741000 962000 965500; do
  J=$KK/prover/block_$h.json
  export HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=4 HAZYNC_COINLOG_TXCOUNTS=$KK/experimental/libzync-bench/cost/txcounts.txt
  ( cd $KK/prover && taskset -c 4-7 nice $W/host_r0 libzync-hints $X/$h.r0.lzb > $X/h$h.r0.log 2>&1 && HAZYNC_LZ_HINTS=$X/$h.r0.lzb taskset -c 4-7 nice $W/host_r0 engine-export $X/r0_$h > $X/x$h.r0.log 2>&1 ) || { say "$h R0_FAIL"; tail -3 $X/x$h.r0.log; continue; }
  ( cd $KK/prover && HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$W/keys-def taskset -c 4-7 nice $W/host_e libzync-hints $X/$h.e.lzb > $X/h$h.e.log 2>&1 && HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$W/keys-def HAZYNC_LZ_HINTS=$X/$h.e.lzb taskset -c 4-7 nice $W/host_e engine-export $X/e_$h > $X/x$h.e.log 2>&1 ) || { say "$h ENGINE_FAIL"; tail -3 $X/x$h.e.log; continue; }
  unset HAZYNC_BLOCK
  r=$(cmp -s $X/$h.r0.lzb $X/$h.e.lzb && echo "bundle same" || echo "bundle DIFFERS")
  for f in $(cd $X/r0_$h && ls | grep -v "cycles$\|method_id$\|^engine$\|agg.r0journal"); do
    if [ "$f" = agg.in ] || [ "$f" = agg_collect.in ]; then
      python3 -c "import sys;a=open(sys.argv[1],'rb').read();b=open(sys.argv[2],'rb').read();sys.exit(0 if len(a)==len(b) and a[:4]==b[:4] and a[36:]==b[36:] else 1)" $X/r0_$h/$f $X/e_$h/$f && r="$r; $f same (but bytes 4..36, METHOD_ID)" || r="$r; $f DIFFERS"
    else cmp -s $X/r0_$h/$f $X/e_$h/$f && r="$r; $f same" || r="$r; $f DIFFERS"; fi
  done
  say "block $h: $r"
done
say C_DONE
