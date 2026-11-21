#!/bin/bash
# pod_run.sh (on the pod, /workspace): REAL proofs (no dev mode) with the v0.27.0 release CUDA host, one card.
#   0. the release host: downloaded from the v0.27.0 release, its sha256 checked against SHA256SUMS.txt, whose own sha256
#      (want_sums.txt) the laptop took from the PGP-verified file; method-id must be 46666b6f.
#   1. warm-up: leaf 170 once (a first prove may JIT CUDA kernels; its time is reported apart, not used for rates)
#   2. (A) the 9 real-state leaves as a board prover proves them: libzync-prove-range h h <dir> leaf <out>, HAZYNC_LEAF_BUNDLE
#   3. (B) 700,000 and 976,000 from their block files: libzync-hints, prove-chunk per chunk, agg-chunks (it proves the trie
#      update against the MODELLED trie first, then the aggregate on the MODELLED coin log: a DEMO block proof).
# Shipped chunking (HAZYNC_CHUNKS unset = 2), the CUDA host's default po2 (21). Each step is timed (date +%s.%N) into
# steps.tsv. A failed prove is retried once (a GPU fault, hazync#119). Ends with out/RUN_DONE.
W=/workspace; O=$W/out; I=$W/in; mkdir -p $O; cd $W || exit 1
unset RISC0_DEV_MODE HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_SEG_PO2
H=$W/hazync-host-cuda
step() {  # step <label> <log> <cmd...>: time one command (retried once on failure)
  local lab=$1 log=$2; shift 2
  for try in 1 2; do
    local t0=$(date +%s.%N); "$@" >> "$log" 2>&1; local rc=$?; local t1=$(date +%s.%N)
    echo -e "$lab\t$try\t$rc\t$(python3 -c "print(round($t1-$t0,1))")\t$(date -u +%T)" >> $O/steps.tsv
    [ $rc = 0 ] && return 0
  done; return 1; }
{ nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader; nproc; grep -m1 'model name' /proc/cpuinfo; } > $O/pod.txt
for f in hazync-host-x86_64-linux-gnu-cuda hazync-verify-x86_64-linux-gnu SHA256SUMS.txt; do
  for t in 1 2 3; do [ -s $f ] && break; curl -fsSL -o $f.tmp https://github.com/hazync/hazync/releases/download/v0.27.0/$f && mv $f.tmp $f; done; done
[ "$(sha256sum SHA256SUMS.txt | cut -c1-64)" = "$(cat want_sums.txt)" ] || { echo "SUMS_BAD" > $O/RUN_DONE; exit 1; }
sha256sum -c --ignore-missing SHA256SUMS.txt > $O/sha.txt 2>&1 || { echo "SHA_BAD" > $O/RUN_DONE; exit 1; }
chmod +x hazync-host-x86_64-linux-gnu-cuda hazync-verify-x86_64-linux-gnu; ln -sf hazync-host-x86_64-linux-gnu-cuda hazync-host-cuda
$H method-id > $O/method_id.txt 2>&1; grep -q 46666b6f7e3d1c1c $O/method_id.txt || { echo "MID_BAD" > $O/RUN_DONE; exit 1; }
( nvidia-smi --query-gpu=utilization.gpu,memory.used,power.draw --format=csv,noheader -l 5 > $O/smi.log 2>/dev/null ) &
leaf() {  # leaf <h> <out name>
  ( cd $O && HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_$1.bin step "leaf $1 $2" $O/leaf_$2.log $H libzync-prove-range $1 $1 unused leaf $2 )
}
echo "WARMUP" > $O/phase; leaf 170 warm170
echo "A" > $O/phase
for h in ${A_HEIGHTS:-170 1000 100000 200000 270000 280000 290000 300000 400000}; do rm -rf $O/o$h; leaf $h o$h; done
echo "B" > $O/phase
for h in ${B_HEIGHTS:-700000 976000}; do
  D=$O/b$h; mkdir -p $D/receipts; cd $D
  export HAZYNC_BLOCK=$I/blocks/block_$h.json HAZYNC_COINLOG_TXCOUNTS=$I/txcounts.txt
  unset HAZYNC_LZ_HINTS
  step "B $h hints" $D/hints.log $H libzync-hints $D/b.lzb || continue
  export HAZYNC_LZ_HINTS=$D/b.lzb
  N=$(sed -n 's/^libzync-hints: block [0-9]* \([0-9]*\) chunks.*/\1/p' $D/hints.log | tail -1); echo "$N" > $D/nchunks
  ok=1; for i in $(seq 0 $((${N:-2}-1))); do
    HAZYNC_OUT=$D/receipts/chunk_$i.hzk step "B $h chunk $i" $D/chunk_$i.log $H prove-chunk $i || { ok=0; break; }
  done
  [ $ok = 1 ] && HAZYNC_RECEIPTS=$D/receipts HAZYNC_AGG_OUT=$D/block_$h.receipt step "B $h agg-chunks" $D/agg.log $H agg-chunks
  unset HAZYNC_BLOCK HAZYNC_LZ_HINTS
done
cd $O; echo "OK" > $O/RUN_DONE
