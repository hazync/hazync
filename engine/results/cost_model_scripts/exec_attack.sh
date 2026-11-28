#!/bin/bash
# ROADMAP 5.1: exec_splits.sh's attack-block step, without the aggregate. engine-export writes every chunk_NN.in, then
# collects the aggregate's ECMH (24,374 inputs of mem_k1140, unhinted: 17 h on one thread and not finished on the first
# attempt); the chunk costs do not need it, so the export is stopped once the log has every chunk ("exported:" lines =
# the "predicted cost ... N chunks" count) and the chunks are executed on keys-rtchb as in exec_splits.sh.
# Run on hazync-coord, after exec_splits.sh built /root/cm51/trt: bash exec_attack.sh > exec_attack.out 2>&1
source ~/.cargo/env; for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
W=/root/cm51; E=/root/engine-kit/engine/host/target/release/hzengine; H=$W/trt/release/host; KEYS=/root/engine/keys-rtchb
ENVS="HAZYNC_NETWORK=regtest HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_PROFILE_LAYOUT=cost HAZYNC_HINT_THREADS=16"
cost() { sed 's/\x1b\[[0-9;]*m//g' "$1" | grep -a "^execute:" | sed -E 's/.*cost ([0-9]+).*/\1/' | head -1; }
for x in ${ATT-mem_k1140:126:/root/worst-case/run/blocks tap_small:127:/root/oomfix/tap/blocks tap_full:127:/root/oomfix/tap/blocks}; do
  IFS=: read n h d <<< "$x"; J=$d/$n/block_$h.json
  for m in ${MODELS:-v1 new}; do
    X=$W/$n.$m; L=$W/$n.$m.export.log
    if [ $m = v1 ]; then CM=HAZYNC_COST_MODEL=v1; else CM=HAZYNC_UNUSED=1; fi
    want=$(grep -a "predicted cost" $L 2>/dev/null | sed -E 's/.* ([0-9]+) chunks.*/\1/' | head -1)
    if [ -z "$want" ] || [ "$(grep -ac 'exported:' $L)" != "$want" ]; then
      rm -rf $X
      [ -f $W/$n.$m.lzb ] || ( cd $W/src/prover && env $ENVS $CM HAZYNC_BLOCK=$J $H libzync-hints $W/$n.$m.lzb > $W/$n.$m.hints.log 2>&1 ) \
        || { echo "$n $m HINTS_FAIL"; continue; }
      ( cd $W/src/prover && exec env $ENVS $CM HAZYNC_BLOCK=$J HAZYNC_LZ_HINTS=$W/$n.$m.lzb $H engine-export $X > $L 2>&1 ) & P=$!
      while kill -0 $P 2>/dev/null; do
        want=$(grep -a "predicted cost" $L | sed -E 's/.* ([0-9]+) chunks.*/\1/' | head -1)
        [ -n "$want" ] && [ "$(grep -ac 'exported:' $L)" = "$want" ] && { sleep 5; kill $P; break; }
        sleep 10; done; wait $P 2>/dev/null
      grep -aq "panicked" $L && { echo "$n $m EXPORT_FAIL: $(grep -a panicked -A1 $L | head -2 | tr '\n' ' ' | cut -c1-160)"; continue; }
    fi
    ls $X/chunk_*.in | xargs -P 8 -I{} sh -c 'b=${1%.in}; '"$E"' execute --dir '"$KEYS"' --input $1 --journal $b.j > $b.exec.log 2>&1; echo $? > $b.rc' _ {}
    line="" costs="" bad=0 same=0 n_c=0
    for f in $X/chunk_*.in; do b=${f%.in}; c=$(cost $b.exec.log); c=${c:-0}; r=$(cat $b.rc); n_c=$((n_c+1))
      [ "$r" = 0 ] || bad=$((bad+1)); [ -f $b.r0journal ] && cmp -s $b.j $b.r0journal && same=$((same+1))
      costs="$costs $c"; line="$line $(basename $b | sed 's/chunk_//'):$c"; done
    st=$(python3 -c "import sys; c=[int(x) for x in sys.argv[1:]]; print(f'total {sum(c):,} max {max(c):,} mean {sum(c)/len(c):,.0f} straggler {max(c)/(sum(c)/len(c)):.2f}')" $costs)
    echo "$n $m: $n_c chunks ($(grep -a 'predicted cost' $L | head -1 | sed 's/^ *//')), exit!=0 $bad, journals = RISC Zero's $same/$n_c; $st"
    echo "   per chunk:$line"
  done
done
echo EXEC_ATTACK_DONE
