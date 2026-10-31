#!/bin/bash
# Pod-side: prove signet LO..HI as one recursive chain on this card, then verify the last receipt against the hints file.
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
LO=${1:?lo}; HI=${2:?hi}; S=/workspace/sb; O=/workspace/out; mkdir -p $O
H=/workspace/hazync/prover/target/release/host
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-1} HAZYNC_SS_HINTS=$S/signet.hints
nvidia-smi --query-gpu=name --format=csv,noheader > $O/card.txt
$H libzync-prove-range $LO $HI $S/json $S/anchor.json $O > $O/prove.log 2>&1; echo "REAL_EXIT=$?" >> $O/prove.log
# anchor_effective.json, not anchor.json: with only LO..HI in the bundle, prove-range seeds the BIP30 tree from the
# coins the range spends and records that root in the anchor it proves from. The round-4 run used anchor.json here
# and its verify FAILED on anchor_id (verify_pod.log); this line was changed AFTER that run (review 5).
$H libzync-verify $O/block_$HI.rcpt $S/signet.hints $O/anchor_effective.json > $O/verify.log 2>&1; echo "REAL_EXIT=$?" >> $O/verify.log
echo RUN_DONE >> $O/prove.log
