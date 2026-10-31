#!/bin/bash
# Pod-side: prove signet LO..HI as one recursive chain on this card, then verify the last receipt against the hints file.
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
LO=${1:?lo}; HI=${2:?hi}; S=/workspace/sb; O=/workspace/out; mkdir -p $O
H=/workspace/hazync/prover/target/release/host
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-1} HAZYNC_SS_HINTS=$S/signet.hints
nvidia-smi --query-gpu=name --format=csv,noheader > $O/card.txt
$H libzync-prove-range $LO $HI $S/json $S/anchor.json $O > $O/prove.log 2>&1; echo "REAL_EXIT=$?" >> $O/prove.log
# Verify against the TRUSTED anchor (the laptop sweep's state), never the prover-written anchor_effective.json:
# that check is circular (review 5). With blocks 1..LO-1 in the bundle the tree is replayed, so the two are equal.
# History: the round-4 run shipped only LO..HI, its seeded tree made this check FAIL on anchor_id (verify_pod.log),
# and the receipt was then verified against anchor_effective.json on the laptop (verify_local.log).
$H libzync-verify $O/block_$HI.rcpt $S/signet.hints $S/anchor.json > $O/verify.log 2>&1; echo "REAL_EXIT=$?" >> $O/verify.log
echo RUN_DONE >> $O/prove.log
