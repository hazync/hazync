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
# R1-5 #3 runs3 (28.11.2026): the host rebuilt with a TEST-ONLY bypass (HZ_TEST_HANDOVER_STATE: the host builds its
# witnesses from the genuine state at 199 and skips its own two checks), so the GUEST sees the bogus predecessor input.
export HZ_TEST_HANDOVER_STATE=$P/block_199.ovm.journal
# control again, on the rebuilt host with the bypass set: the genuine predecessor must still pass the collect pass
case_ "control (bypass host): the genuine predecessor proof of 199 (keys-xv)" $W/keys-xv "$(mk control3)" control
# 4. a genuine predecessor proof at another height (keys-ch's block 200 for 199)
d=$(mk other_height3)
$E wasm-inputs --dir /root/engine/keys-ch --proof /root/engine/core200/block_200.ovm --out $d/wi > $d/wi.log 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin
cp /root/engine/core200/block_200.ovm.journal $d/block_199.ovm.journal
case_ "4. a genuine predecessor proof at height 200, not 199 (keys-xv, bypass host)" $W/keys-xv $d
# 8. a non-chain journal: a genuine chunk proof of keys-ch (block 130,000)
d=$(mk non_chain3)
$E wasm-inputs --dir /root/engine/keys-ch --proof $W/c8/chunk --out $d/wi > $d/wi.log 2>&1 && cp $d/wi/proof.bin $d/block_199.ovm.bin \
  && cp $W/c8/chunk.journal $d/block_199.ovm.journal || { echo "8. INPUT_FAIL"; tail -3 $d/wi.log; }
case_ "8. a non-chain journal: a genuine chunk proof of keys-ch (block 130,000) (keys-xv, bypass host)" $W/keys-xv $d
echo RUNS3_DONE
