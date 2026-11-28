#!/bin/bash
# R1-5 #3 runs (28.11.2026), inside a private mount namespace like setup.sh. Each case: block 200 as the handover block
# from a predecessor input, under one test image; execute only. The guest's ovm-xver / handover checks run in the
# aggregate's collect pass (s0/00001_b200_agg_collect); a refusal there is the guest's (its panic message is recorded).
# The control (the genuine predecessor under keys-xv) must pass that pass: it is stopped as soon as the next step starts.
set -uo pipefail
for v in $(compgen -e | grep "^HAZYNC_\|^HZ_"); do unset "$v"; done
W=/root/hvneg28; mount --bind $W/kit /root/engine-kit && mount --bind $W/src-xv /root/engine-src-xv || { echo MOUNT_FAIL; exit 1; }
E=/root/engine-kit/engine/host/target/release/hzengine; HX=/root/engine-src-xv/prover/target/release/host
P=/root/engine/xvpred; N=$W/cases; mkdir -p $N; cd $W || exit 1
TX=/root/engine-kit/experimental/libzync-bench/cost/txcounts.txt
case_() { # <label> <keys dir> <input dir> [control]
  local d=$3 t0=$(date +%s); rm -rf "${d:?}/out"
  ( HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$2 HAZYNC_HANDOVER_PROOF=$d/block_199.ovm HAZYNC_COINLOG_TXCOUNTS=$TX \
      exec setsid taskset -c 12-15 nice $HX libzync-prove-range 200 200 /root/engine/xchain/blocks handover $d/out ) > $d/run.log 2>&1 &
  local pid=$! rc= c=$d/out/engine/s0/00001_b200_agg_collect.log
  while kill -0 $pid 2>/dev/null; do
    if [ -n "${4:-}" ] && ls $d/out/engine/s0/00002_* > /dev/null 2>&1; then
      kill -TERM -- -$pid 2>/dev/null; sleep 2; kill -KILL -- -$pid 2>/dev/null; rc=stopped; break; fi
    sleep 5
  done
  [ -z "$rc" ] && { wait $pid; rc=$?; }
  local g=$(sed 's/\x1b\[[0-9;]*m//g' $c 2>/dev/null | grep -a -A1 "panicked at" | grep -av "^--$" | head -2 | tr '\n' ' ' | cut -c1-260)
  local h=$(sed 's/\x1b\[[0-9;]*m//g' $d/run.log | grep -a "panicked at\|rror" | head -2 | tr '\n' ' ' | cut -c1-200)
  local s=$(( $(date +%s)-t0 ))
  if [ "$rc" = stopped ]; then
    if [ -z "$g" ] && grep -aq "guest exit" $c; then echo "PASSED-COLLECT $1 (${s} s; stopped once the next step started)"; else echo "CONTROL-UNCLEAR $1 :: $g :: $h"; fi
  elif [ "$rc" != 0 ] && [ -n "$g" ]; then echo "refused-in-guest $1 (rc $rc, ${s} s): $g"
  elif [ "$rc" != 0 ]; then echo "refused-NOT-in-guest $1 (rc $rc, ${s} s): $h"
  else echo "ACCEPTED $1 (rc 0, ${s} s)"; fi
}
mk() { local d=$N/$1; rm -rf "$d"; mkdir -p $d; cp $P/block_199.ovm.journal $P/block_199.ovm.bin $P/state_199.coinlog $P/state_199.trie $d/; echo $d; }
# control: the genuine predecessor (keys-ch's proof of 199) under keys-xv
case_ "control: the genuine predecessor proof of 199 (keys-xv)" $W/keys-xv "$(mk control)" control
# 4. a genuine proof of the predecessor image at another height (keys-ch's block 200 for 199)
d=$(mk other_height)
$E wasm-inputs --dir /root/engine/keys-ch --proof /root/engine/core200/block_200.ovm --out $d/wi > $d/wi.log 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin
cp /root/engine/core200/block_200.ovm.journal $d/block_199.ovm.journal
case_ "4. a genuine predecessor proof at height 200, not 199 (keys-xv)" $W/keys-xv $d
# 5. the carried key's bytes are not its pin's (image k5)
case_ "5. the embedded key file is not the pinned key (image k5)" $W/keys-k5 "$(mk key_not_pin)"
# 6. another image's genuine key under the predecessor's pin entry (image k6)
case_ "6. the lineage carries another image's genuine key (image k6)" $W/keys-k6 "$(mk other_key)"
# 7. the lineage states a history the predecessor does not have (image k7)
case_ "7. predecessor lineage mismatch (image k7, two-entry lineage)" $W/keys-k7 "$(mk lineage_mismatch)"
# 8. a non-chain journal: a GENUINE chunk proof of the predecessor image (keys-ch proved block 130,000 chunk 0, c8/)
#    as the predecessor proof (the review asks a kind check to refuse a journal that is not a chain state)
d=$(mk non_chain)
$E wasm-inputs --dir /root/engine/keys-ch --proof $W/c8/chunk --out $d/wi > $d/wi.log 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin \
  && cp $W/c8/chunk.journal $d/block_199.ovm.journal || { echo "8. INPUT_FAIL"; tail -3 $d/wi.log; }
case_ "8. a non-chain journal: a genuine chunk proof of keys-ch (block 130,000) (keys-xv)" $W/keys-xv $d
echo RUNS_DONE
