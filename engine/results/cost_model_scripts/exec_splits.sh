#!/bin/bash
# ROADMAP 5.1 (27.11.2026): the chunk splits of the v1 and the new cost model (prover/host chunks.rs, 909da897), both
# EXPORTED by today's product host (HAZYNC_COST_MODEL=v1 gives back the old model exactly) and EXECUTED (no proving) on
# the same images the measured v1 costs came from: keys-ch for the mainnet edge blocks (edgef.sh), keys-rtchb (regtest,
# metering) for the attack blocks (guest_memory/heap_per_chunk.sh). Per chunk: the engine's execute cost and exit;
# per block: max / mean (the straggler). Run on hazync-coord: bash exec_splits.sh > exec_splits.out 2>&1
# (subsets: ORD="<heights>" ATT="" MODELS=v1 bash exec_splits.sh)
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
W=/root/cm51; E=/root/engine-kit/engine/host/target/release/hzengine; K=/root/engine
mkdir -p $W && cd $W || exit 1
[ -d src ] || git clone -q --depth 1 --branch engine/openvm https://github.com/hazync/hazync.git src
echo "source $(git -C src log --oneline -1 | cut -c1-80)"
for net in main rt; do
  T=$W/t$net; [ -x $T/release/host ] && continue
  s=$(date +%s)
  if [ $net = rt ]; then N="HAZYNC_NETWORK=regtest"; else N=""; fi
  (cd src/prover && env $N HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 CARGO_TARGET_DIR=$T nice cargo build --release -p host > $W/build_$net.log 2>&1) \
    || { echo "BUILD_FAIL $net"; grep -a "^error" -A6 $W/build_$net.log | head; exit 1; }
  echo "host $net built in $(( $(date +%s) - s )) s"
done
cost() { sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -a "^execute:" | sed -E 's/.*cost ([0-9]+).*/\1/' | head -1; }
run() { # <name> <host> <block json> <keys> <extra env...>
  local n=$1 H=$2 J=$3 KEYS=$4; shift 4
  for m in ${MODELS:-v1 new}; do
    X=$W/$n.$m; rm -rf $X
    if [ $m = v1 ]; then CM=HAZYNC_COST_MODEL=v1; else CM=HAZYNC_UNUSED=1; fi
    # hints PER MODEL: under HAZYNC_PROFILE_LAYOUT=cost a hint bundle records its chunk layout and is ignored by a chunk
    # of another layout (lz.rs), so v1 chunks fed the new model's hints run unhinted (first run: 4-34x the cost)
    ( cd $W/src/prover && env "$@" $CM HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=16 $H libzync-hints $W/$n.$m.lzb > $W/$n.$m.hints.log 2>&1 ) \
      || { echo "$n $m HINTS_FAIL"; continue; }
    ( cd $W/src/prover && env "$@" $CM HAZYNC_BLOCK=$J HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=16 HAZYNC_LZ_HINTS=$W/$n.$m.lzb $H engine-export $X > $W/$n.$m.export.log 2>&1 ) \
      || { echo "$n $m EXPORT_FAIL: $(grep -a 'panicked\|INVALID' $W/$n.$m.export.log | head -1 | cut -c1-120)"; continue; }
    ls $X/chunk_*.in | xargs -P 8 -I{} sh -c 'b=${1%.in}; '"$E"' execute --dir '"$KEYS"' --input $1 --journal $b.j > $b.exec.log 2>&1; echo $? > $b.rc' _ {}
    local line="" costs="" bad=0 same=0 n_c=0
    for f in $X/chunk_*.in; do b=${f%.in}; c=$(cost $b.exec.log); c=${c:-0}; r=$(cat $b.rc); n_c=$((n_c+1))
      [ "$r" = 0 ] || bad=$((bad+1)); [ -f $b.r0journal ] && cmp -s $b.j $b.r0journal && same=$((same+1))
      costs="$costs $c"; line="$line $(basename $b | sed 's/chunk_//'):$c"; done
    st=$(python3 -c "import sys; c=[int(x) for x in sys.argv[1:]]; print(f'total {sum(c):,} max {max(c):,} mean {sum(c)/len(c):,.0f} straggler {max(c)/(sum(c)/len(c)):.2f}')" $costs)
    echo "$n $m: $n_c chunks, exit!=0 $bad, journals = RISC Zero's $same/$n_c; $st"
    echo "   per chunk:$line"
  done
}
for h in ${ORD-91880 185956 227931 364292 388381 419328 481824}; do
  J=/root/engine-src/prover/testdata/boundaries/block_$h.json; [ -f $J ] || J=/root/engine-src/prover/block_$h.json
  [ -f $J ] || { echo "$h NO_BLOCK_JSON"; continue; }
  run $h $W/tmain/release/host $J $K/keys-ch HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000 HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt
done
for x in ${ATT-mem_k1140:126:/root/worst-case/run/blocks tap_small:127:/root/oomfix/tap/blocks tap_full:127:/root/oomfix/tap/blocks}; do
  IFS=: read n h d <<< "$x"
  # the host now synthesises coin-log positions from getchaintxstats pairs (cl_synth); the regtest blocks need the file too
  MODELS="v1 new" run $n $W/trt/release/host $d/$n/block_$h.json $K/keys-rtchb HAZYNC_NETWORK=regtest HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt
done
echo EXEC_SPLITS_DONE
