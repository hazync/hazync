#!/bin/bash
# Pod-side: the zAPP regtest chain proved for real (no dev mode) on the regtest image, as scripts/zapp-regtest/execute.sh
# executed it: A = 1..112 from genesis over main + a; B = A's output cut back to 110, resumed over main + b to 113.
# Every height's coin log and trie snapshot is kept (HAZYNC_COINLOG_SAVE_EACH=1: the chain is small) so the zAPP tests
# can anchor at any height. A prover panic (a GPU fault) does not end the run: it goes again from the newest receipt.
export PATH=/usr/local/cuda-12.8/bin:$HOME/.cargo/bin:$PATH CUDA_PATH=/usr/local/cuda-12.8 LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64
S=/workspace/sb; O=/workspace/out; mkdir -p $O
H=/workspace/hazync/prover/target/release/host
unset RISC0_DEV_MODE
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_SAVE_EACH=1 HAZYNC_RESUME=1
nvidia-smi --query-gpu=name --format=csv,noheader > $O/card.txt
$H method-id | head -1 > $O/method_id.txt
prove() {  # prove <hi> <json dir> <out dir> <log>
  for try in 1 2 3 4 5; do
    $H libzync-prove-range 1 $1 $2 genesis $3 >> $4 2>&1; rc=$?; echo "REAL_EXIT=$rc (try $try)" >> $4
    [ $rc = 0 ] && return 0
  done; return 1; }
prove 112 $S/json_a $O/a $O/prove_a.log || { echo "A FAILED" > $O/RUN_DONE; exit 1; }
# B from A's 110: every per-height file above 110 removed (resuming on A's 112 is refused: execute.sh's control)
rm -rf $O/b; cp -r $O/a $O/b
for f in $O/b/block_*.rcpt $O/b/state_*.* $O/b/bundle_*.lzb; do
  [ -e "$f" ] || continue; h=$(basename "$f" | sed -E 's/^[a-z]+_([0-9]+)\..*/\1/'); [ "$h" -gt 110 ] && rm -f "$f"; done
prove 113 $S/json_b $O/b $O/prove_b.log || { echo "B FAILED" > $O/RUN_DONE; exit 1; }
for x in a:112 b:113; do
  $H libzync-verify $O/${x%:*}/block_${x#*:}.rcpt > $O/verify_${x%:*}.log 2>&1; echo "REAL_EXIT=$?" >> $O/verify_${x%:*}.log
done
echo "OK" > $O/RUN_DONE
