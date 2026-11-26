#!/bin/bash
# Stage 2.3: the zAPP hosts' single-source inputs + --stream (commit "zAPP hosts: --stream ..."). On the engine chain 1..200
# (core200: the engine's block-200 proof and its state JSON; xchain/blocks: mainnet blocks 1..200).
# (1) RISC Zero path unchanged: the OLD zyncapp (12:41 build) and the NEW one, executed on the same inputs, print the
#     same statements and cycles; hints' --stream bytes identical.
# (2) The engine: utreexo base to 200 from the new --stream, executed with the core proof as its child; its journal must
#     be RISC Zero's (from (1)) except the statement's self_id (8 words at bytes 4..36: kind, then self_id).
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done; unset RISC0_DEV_MODE
E=/root/engine-kit/engine/host/target/release/hzengine; C=/root/engine/core200; B=/root/engine/xchain/blocks
T=/root/engine/zt; mkdir -p $T; cd /root/engine || exit 1
[ -f $T/zyncapp_old ] || cp /root/engine-src/prover/target/release/zyncapp $T/zyncapp_old
cd /root/engine-src && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD && git log --oneline -1 | cut -c1-60
cd prover && s=$(date +%s); HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 nice cargo build --release -p zyncapp-host > $T/build.log 2>&1 || { echo BUILD_FAIL; grep -a "^error" -A6 $T/build.log | head; exit 1; }
echo "zyncapp built in $(( $(date +%s)-s )) s"; cp target/release/zyncapp $T/zyncapp_new; cd /root/engine
O=$T/zyncapp_old; N=$T/zyncapp_new
strip_t() { sed -E 's/ in [0-9.]+ s//; s/\([0-9.]+ s\)//; s/executed in [0-9.]+ s//' "$1" | grep -a "^UTREEXO\|^HINTS"; }
# hints stream
$O hints-prove $B 200 $C/state_200.json $T/h_old --stream $T/h_old.bin > $T/h_old.log 2>&1
$N hints-prove $B 200 $C/state_200.json $T/h_new --stream $T/h_new.bin > $T/h_new.log 2>&1
cmp -s $T/h_old.bin $T/h_new.bin && echo "hints stream: IDENTICAL ($(stat -c %s $T/h_new.bin) bytes)" || echo "hints stream: DIFFERENT"
# utreexo base, RISC Zero execute, old vs new, to 100 and to 200
for h in 100 200; do
  $O utreexo-prove $B $h $C/state_200.json $T/u_old_$h --execute > $T/u_old_$h.log 2>&1; ro=$?
  $N utreexo-prove $B $h $C/state_200.json $T/u_new_$h --execute > $T/u_new_$h.log 2>&1; rn=$?
  cmp -s $T/u_old_$h/mainnet-$h.journal $T/u_new_$h/mainnet-$h.journal && jm=IDENTICAL || jm=DIFFERENT
  [ "$(strip_t $T/u_old_$h.log)" = "$(strip_t $T/u_new_$h.log)" ] && lm=same || lm=DIFFER
  echo "utreexo base to $h (RISC Zero execute): rc old $ro new $rn; journal $jm; statement + cycle lines $lm"
done
# utreexo tip step 100 -> 200, RISC Zero execute, old vs new
$O utreexo-tip-prove $B $T/u_old_100/mainnet-100.journal 200 $C/state_200.json $T/t_old --execute > $T/t_old.log 2>&1; ro=$?
$N utreexo-tip-prove $B $T/u_new_100/mainnet-100.journal 200 $C/state_200.json $T/t_new --execute > $T/t_new.log 2>&1; rn=$?
cmp -s $T/t_old.journal $T/t_new.journal && jm=IDENTICAL || jm=DIFFERENT
[ "$(strip_t $T/t_old.log)" = "$(strip_t $T/t_new.log)" ] && lm=same || lm=DIFFER
echo "utreexo tip 100 -> 200 (RISC Zero execute): rc old $ro new $rn; journal $jm; lines $lm"
# the engine: utreexo base to 200
$N utreexo-prove $B 200 $C/state_200.json $T/u_eng --stream $T/u200.bin > $T/u_stream.log 2>&1; echo "stream rc=$?: $(grep -a 'UTREEXO_STREAM' $T/u_stream.log | cut -c1-200)"
s=$(date +%s); HZ_CHILD_KEYS=/root/engine/keys-ch $E execute --dir keys-z-utreexo --input $T/u200.bin --self-id-at 0 --child $C/block_200.ovm --journal $T/u200.j --log $T/u200.guest.log > $T/u200.exec.log 2>&1; rc=$?
echo "engine execute rc=$rc $(( $(date +%s)-s )) s :: $(sed 's/\x1b\[[0-9;]*m//g' $T/u200.exec.log | grep -a '^execute\|rror\|EXECUTE' | head -2 | tr '\n' ' ' | cut -c1-200)"
python3 - $T/u200.j $T/u_new_200/mainnet-200.journal <<'PY'
import sys
a, b = open(sys.argv[1], "rb").read(), open(sys.argv[2], "rb").read()
d = [i for i in range(min(len(a), len(b))) if a[i] != b[i]]
print(f"engine journal {len(a)} bytes vs RISC Zero {len(b)}; differing bytes at {d[0] if d else '-'}..{d[-1] if d else '-'} ({len(d)}); "
      + ("ONLY the statement's self_id (bytes 4..36)" if len(a) == len(b) and d and d[0] >= 4 and d[-1] < 36 else "CHECK"))
PY
echo ZT_DONE
