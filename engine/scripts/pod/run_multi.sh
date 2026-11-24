#!/bin/bash
# On the pod: (1) the engine proves every exported block (prove-block, in-guest recursion); (2) the RISC Zero product
# (released v0.26.0 CUDA host, image 46666b6f) proves the blocks listed in R0_BLOCKS on the SAME card, as
# experimental/fresh-cost-model/bench/pod_hazync.sh did for 976,379. One times.tsv line per block and engine.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=host/target/release/hzengine; T=out/times.tsv
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 2 > out/smi.log ) & SMI=$!
for X in export/*/; do
  h=$(basename $X); s=$(date +%s)
  $H prove-block --dir keys --export $X --out out/ovm_$h > out/ovm_$h.log 2>&1; rc=$?
  echo -e "ovm\t$h\t$rc\t$(( $(date +%s)-s ))" >> $T
  $H verify --dir keys --proof out/ovm_$h/block > out/ovm_${h}_verify.log 2>&1; echo -e "ovm_verify\t$h\t$?" >> $T
  rm -f out/ovm_$h/chunk_*.proof.json out/ovm_$h/trie.proof.json   # keep the block proof, journals, report
done
B=https://github.com/hazync/hazync/releases/download/v0.26.0
curl -sSfLO $B/hazync-host-x86_64-linux-gnu-cuda && curl -sSfLO $B/SHA256SUMS.txt && \
  grep " hazync-host-x86_64-linux-gnu-cuda$" SHA256SUMS.txt | sha256sum -c - > out/r0_sha.txt 2>&1 || echo "r0 download FAILED" >> $T
R=/workspace/hazync-host-x86_64-linux-gnu-cuda; chmod +x $R; $R method-id 2>&1 | tail -1 > out/r0_method_id.txt
for h in ${R0_BLOCKS:-130000 140000 741000 962000}; do
  O=/workspace/r0_$h; mkdir -p $O/receipts; cd /workspace/rblocks
  export HAZYNC_BLOCK=/workspace/rblocks/block_$h.json HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 \
         HAZYNC_COINLOG_TXCOUNTS=/workspace/rblocks/hz_txcounts.txt HAZYNC_HINT_THREADS=$(nproc)
  $R libzync-hints $O/b.lzb > $O/hints.log 2>&1 || { echo -e "r0\t$h\thints-failed" >> /workspace/$T; continue; }
  export HAZYNC_LZ_HINTS=$O/b.lzb HAZYNC_RECEIPTS=$O/receipts
  NCH=$(sed -n 's/^libzync-hints: block [0-9]* \([0-9]*\) chunks.*/\1/p' $O/hints.log | tail -1)
  s=$(date +%s); rc=0
  for i in $(seq 0 $((${NCH:-0}-1))); do HAZYNC_OUT=$O/receipts/chunk_$i.hzk $R prove-chunk $i > $O/prove_$i.log 2>&1 || rc=1; done
  c=$(( $(date +%s)-s )); s=$(date +%s)
  HAZYNC_AGG_OUT=$O/block.receipt $R agg-chunks > $O/agg.log 2>&1 || rc=2
  echo -e "r0\t$h\t$rc\t$(( c + $(date +%s)-s ))\tchunks=$NCH chunk_s=$c agg_s=$(( $(date +%s)-s ))" >> /workspace/$T
  unset HAZYNC_LZ_HINTS HAZYNC_RECEIPTS; cd /workspace; cp $O/*.log out/ 2>/dev/null; mkdir -p out/r0_$h; cp $O/*.log out/r0_$h/
done
kill $SMI; awk -F', ' '{gsub(/ %/,"",$1); s+=$1; n++} END {print "gpu util mean", s/n}' out/smi.log >> out/gpu.txt
echo "rc=0" > out/RUN_DONE
