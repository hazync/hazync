#!/bin/bash
# Run 2, arm 3 ON the pod: Hazync's OWN product guest (release v0.26.1 CUDA host, image 46666b6f) proving block
# 976,379 on this ONE card, the same steps as experimental/libzync-bench/gpu/gpu-libzync-multi.sh with N=1:
# hints + execute check, every chunk, then the aggregate (trie update proved first). Coin log / trie around the
# block are MODELLED (cl_synth_around, from the block's spends + hz_txcounts.txt), as in the 962,000 runs:
# the cycles are the real block's, the state is not history. Every receipt is verified by the host.
cd /workspace; OUT=/workspace/hz; mkdir -p $OUT/receipts
say(){ echo "[$(date +%H:%M:%S)] $*" | tee -a $OUT/run.log; }
die(){ say "FATAL: $*"; echo HZ_DONE >> $OUT/run.log; exit 1; }
B=https://github.com/hazync/hazync/releases/download/v0.26.1
curl -sSfLO $B/hazync-host-x86_64-linux-gnu-cuda && curl -sSfLO $B/SHA256SUMS.txt || die download
grep " hazync-host-x86_64-linux-gnu-cuda$" SHA256SUMS.txt | sha256sum -c - >> $OUT/run.log 2>&1 || die "sha mismatch"
BIN=/workspace/hazync-host-x86_64-linux-gnu-cuda; chmod +x $BIN
say "method-id: $($BIN method-id 2>&1 | tail -1)"
$BIN method-id 2>&1 | grep -q 46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c || die "not the product image"
export HAZYNC_BLOCK=/workspace/block_976379.json HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 \
       HAZYNC_COINLOG_TXCOUNTS=/workspace/hz_txcounts.txt HAZYNC_HINT_THREADS=$(nproc)
T0=$SECONDS
$BIN libzync-hints $OUT/b.lzb > $OUT/hints.log 2>&1 || die "hints, see hints.log"
export HAZYNC_LZ_HINTS=$OUT/b.lzb
$BIN agg-exec-fake > $OUT/exec.log 2>&1 || die "exec check"
grep -aq REJECT $OUT/exec.log && die "aggregate REJECTED"
say "hints + execute $((SECONDS-T0)) s, aggregate cycles $(grep -ao 'block validation cycles *[0-9]*' $OUT/exec.log | awk '{print $NF}' | tail -1)"
NCH=$(sed -n 's/^libzync-hints: block [0-9]* \([0-9]*\) chunks.*/\1/p' $OUT/hints.log | tail -1); [ -n "$NCH" ] || die "no chunk count"
export HAZYNC_RECEIPTS=$OUT/receipts
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 2 > $OUT/smi.log 2>/dev/null ) & SMI=$!
T0=$SECONDS
for i in $(seq 0 $((NCH-1))); do
  A=$SECONDS; HAZYNC_OUT=$OUT/receipts/chunk_$i.hzk $BIN prove-chunk $i > $OUT/prove_$i.log 2>&1 || { kill $SMI; die "chunk $i"; }
  say "chunk $i $((SECONDS-A)) s"
done
CH=$((SECONDS-T0)); say "RESULT vm=hazync tag=block_976379 step=chunks n=$NCH secs=$CH"
T0=$SECONDS; HAZYNC_AGG_OUT=$OUT/block.receipt $BIN agg-chunks > $OUT/agg.log 2>&1 || { kill $SMI; die "aggregate"; }
AG=$((SECONDS-T0)); kill $SMI
say "RESULT vm=hazync tag=block_976379 step=aggregate secs=$AG"
say "RESULT vm=hazync tag=block_976379 step=block secs=$((CH+AG)) (chunks + aggregate, one card, receipts verified by host)"
echo HZ_DONE >> $OUT/run.log
