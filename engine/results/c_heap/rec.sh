#!/bin/bash
# FEASIBILITY Q7: the `reclaim` prototype (00e88ec0). (1) mainnet image at 2^29: 976,379's chunks + trie vs RISC Zero's
# format-2 journals (xf2) and the cost vs keys-f2 (the same guest on the bump allocator); (2) regtest image + a regtest
# host at this commit; tap_small (block 127's one input ran out of 512 MiB on the bump allocator) through
# libzync-prove-range on the engine, CPU.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
(cd engine/host && nice cargo build --release 2>&1 | tail -1)
E=/root/engine-kit/engine/host/target/release/hzengine; cd /root/engine || exit 1
rm -rf /root/engine-guestrec && /root/engine-kit/engine/guest/assemble.sh /root/engine-guestrec > asmrec.log 2>&1 || { echo ASM_FAIL; tail -5 asmrec.log; exit 1; }
G=/root/engine-guestrec/prover/methods/guest
mk() { k=$1; shift; rm -rf $k && mkdir $k && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} $k/
  env "$@" HZ_BASE=/root/engine-base5 nice $E build --dir $k --guest $G > ${k}_build.log 2>&1 || { echo "$k BUILD_FAIL"; grep -a "^error\|panicked" -A6 ${k}_build.log | head; exit 1; }
  echo "$k: $(grep -a '^build:' ${k}_build.log | cut -c1-110)"; }
mk keys-rec HZ_GUEST_FEATURES=reclaim
X=/root/engine/xf2/976379t
for f in $X/chunk_*.in $X/trie.in; do b=${f%.in}
  a=$(taskset -c 0-15 $E execute --dir keys-f2 --input $f 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:" | sed -E "s/.*cost ([0-9]+).*/\1/")
  r=$(taskset -c 0-15 $E execute --dir keys-rec --input $f --journal $b.rec.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute\|rror\|Maximum\|panicked" | head -1)
  c=$(echo "$r" | sed -E "s/.*cost ([0-9]+).*/\1/")
  cmp -s $b.rec.j $b.r0journal && m=IDENTICAL || m=DIFFERENT
  echo "$(basename $f) $m :: bump $a, reclaim $c ($(python3 -c "print(f'{($c/$a-1)*100:+.1f}%')" 2>/dev/null)) :: $(echo $r | cut -c1-60)"; done
# (2) regtest
mk keys-rtrec HAZYNC_NETWORK=regtest HZ_GUEST_FEATURES=regtest,reclaim
cd /root/engine-src-rt && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD && cd prover
s=$(date +%s); HAZYNC_BASE=/root/product-base HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=regtest nice cargo build --release -p host > /root/engine/rec_host.log 2>&1 || { echo HOST_FAIL; grep -a "^error" -A6 /root/engine/rec_host.log | head; exit 1; }
echo "regtest host $(( $(date +%s)-s )) s"
H=/root/engine-src-rt/prover/target/release/host; T=/root/oomfix/tap/blocks; cd /root/engine || exit 1
rm -rf rec_tap_small; s=$(date +%s)
HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-rtrec taskset -c 0-15 $H libzync-prove-range 126 127 $T/tap_small $T/anchor.json rec_tap_small > rec_tap_small.log 2>&1; rc=$?
echo "tap_small (reclaim, 2^29) rc=$rc $(( $(date +%s)-s )) s :: $(grep -a 'panicked\|re-split\|block 12[67]:' rec_tap_small.log | tail -3 | tr '\n' ' ' | cut -c1-240)"
echo REC_DONE
