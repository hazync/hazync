#!/bin/bash
# Pod-side: prove signet 1..HI from genesis as one recursive chain on the published image, resuming after the
# newest receipt in out/ if there is one, then verify the final receipt with no anchor: OK, or DEMO (exit 2) when the chain is below Core's minimum chainwork
# (every signet test range is: a valid chain from genesis, not shown to be the network's).
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
HI=${1:?hi}; S=/workspace/sb; O=/workspace/out; mkdir -p $O
H=/workspace/hazync/prover/target/release/host
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-1} HAZYNC_RESUME=1
# Design B reads its hints file; a coin-log image has none and is verified with '-'.
if [ -f $S/signet.hints ]; then export HAZYNC_SS_HINTS=$S/signet.hints; VH=$S/signet.hints; else VH=-; fi
nvidia-smi --query-gpu=name --format=csv,noheader > $O/card.txt
# A prover panic (a GPU fault, say) must not end the run: go again from the newest receipt, on this pod.
for try in 1 2 3 4 5; do
  $H libzync-prove-range 1 $HI $S/json genesis $O >> $O/prove.log 2>&1; rc=$?; echo "REAL_EXIT=$rc (try $try)" >> $O/prove.log
  [ $rc = 0 ] && break
done
$H libzync-verify $O/block_$HI.rcpt $VH > $O/verify.log 2>&1; echo "REAL_EXIT=$?" >> $O/verify.log
echo RUN_DONE >> $O/prove.log
