#!/bin/bash
# FEASIBILITY Q20: the hints zyncAPP end to end on the engine. The core: the engine chain 1..200 from genesis
# (pod run_chain.sh; /root/engine/core200: state_200.json, block_200.ovm.{proof.json,journal}, history.bin). The zAPP
# host writes the hints guest's stream (--stream), the engine executes it with the core proof as its child (env::verify)
# and then proves it; the statement is checked against the host's own derivation.
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
E=/root/engine-kit/engine/host/target/release/hzengine; Z=/root/engine-src/prover/target/release/zyncapp
C=/root/engine/core200; B=/root/engine/xchain/blocks; T=/root/engine/hz; rm -rf $T; mkdir -p $T; cd /root/engine || exit 1
$Z hints-prove $B 200 $C/state_200.json $T --stream $T/s200.bin > $T/stream.log 2>&1; echo "stream rc=$?: $(grep -a 'HINTS_STREAM\|HINTS_NATIVE\|error\|die\|panicked' $T/stream.log | tail -2 | cut -c1-220)"
[ -f $T/s200.bin ] || exit 1
s=$(date +%s); HZ_CHILD_KEYS=/root/engine/keys-ch $E execute --dir keys-z-hints --input $T/s200.bin --child $C/block_200.ovm --journal $T/j200.bin > $T/exec.log 2>&1; rc=$?
echo "execute rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' $T/exec.log | grep -a '^execute\|rror\|panicked\|children' | head -3 | tr '\n' ' ' | cut -c1-260)"
[ $rc = 0 ] || exit 1
s=$(date +%s); HZ_CHILD_KEYS=/root/engine/keys-ch nice $E prove --dir keys-z-hints --input $T/s200.bin --child $C/block_200.ovm --out $T/p200 > $T/prove.log 2>&1; rc=$?
echo "prove rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' $T/prove.log | grep -a '^prove\|journal sha\|rror\|panicked' | tail -2 | tr '\n' ' ' | cut -c1-220)"
[ $rc = 0 ] && { $E verify --dir keys-z-hints --proof $T/p200 > $T/verify.log 2>&1; echo "verify rc=$? $(sed 's/\x1b\[[0-9;]*m//g' $T/verify.log | grep -a '^verify' | cut -c1-140)"; }
echo HINTS_DONE
