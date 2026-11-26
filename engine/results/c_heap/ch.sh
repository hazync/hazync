#!/bin/bash
# FEASIBILITY Q7: #794's C-heap recycling on the engine (4cb9e0c1). (1) bases rebuilt (patch 0101's hz_lsh_key);
# (2) mainnet image at 2^29: 976,379's chunks + trie vs RISC Zero's format-2 journals (xf2) and cost vs keys-f2 (the bump
# heap); (3) regtest image + a regtest host at this commit; tap_small (block 127's one input ran out of 512 MiB) and
# light_only through libzync-prove-range on the engine, CPU.
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-50
(cd engine/host && nice cargo build --release 2>&1 | tail -1)
E=/root/engine-kit/engine/host/target/release/hzengine; cd /root/engine || exit 1
s=$(date +%s); rm -rf /root/engine-base6 && /root/engine-kit/engine/guest/base.sh /root/engine-base6 > base6.log 2>&1 || { echo BASE_FAIL; tail -5 base6.log; exit 1; }; echo "engine base $(( $(date +%s)-s )) s"
rm -rf /root/engine-guestch && /root/engine-kit/engine/guest/assemble.sh /root/engine-guestch > asmch.log 2>&1 || { echo ASM_FAIL; tail -5 asmch.log; exit 1; }
G=/root/engine-guestch/prover/methods/guest
mk() { k=$1; shift; rm -rf $k && mkdir $k && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} $k/
  env "$@" HZ_BASE=/root/engine-base6 nice $E build --dir $k --guest $G > ${k}_build.log 2>&1 || { echo "$k BUILD_FAIL"; grep -a "^error\|panicked" -A6 ${k}_build.log | head; exit 1; }
  echo "$k: $(grep -a '^build:' ${k}_build.log | cut -c1-110)"; }
mk keys-ch
X=/root/engine/xf2/976379t
for f in $X/chunk_*.in $X/trie.in; do b=${f%.in}
  a=$(taskset -c 0-15 $E execute --dir keys-f2 --input $f 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:" | sed -E "s/.*cost ([0-9]+).*/\1/")
  r=$(taskset -c 0-15 $E execute --dir keys-ch --input $f --journal $b.ch.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute\|rror\|Maximum\|panicked" | head -1)
  c=$(echo "$r" | sed -E "s/.*cost ([0-9]+).*/\1/")
  cmp -s $b.ch.j $b.r0journal && m=IDENTICAL || m=DIFFERENT
  echo "$(basename $f) $m :: bump $a, c-heap $c ($(python3 -c "print(f'{($c/$a-1)*100:+.1f}%')" 2>/dev/null))"; done
mk keys-rtch HAZYNC_NETWORK=regtest HZ_GUEST_FEATURES=regtest
# the product base too (the regtest host's own guest compiles patch 0101), then the regtest host at this commit
s=$(date +%s); rm -rf /root/product-base6 && /root/engine-kit/scripts/libzync-base.sh /root/product-base6 > pbase6.log 2>&1 || { echo PBASE_FAIL; tail -5 pbase6.log; exit 1; }; echo "product base $(( $(date +%s)-s )) s"
cd /root/engine-src-rt && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD && cd prover
s=$(date +%s); HAZYNC_BASE=/root/product-base6 HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=regtest nice cargo build --release -p host > /root/engine/ch_host.log 2>&1 || { echo HOST_FAIL; grep -a "^error" -A6 /root/engine/ch_host.log | head; exit 1; }
echo "regtest host $(( $(date +%s)-s )) s"
H=/root/engine-src-rt/prover/target/release/host; T=/root/oomfix/tap/blocks; B=/root/worst-case/run/blocks; cd /root/engine || exit 1
run() { nm=$1 lo=$2 hi=$3 blk=$4 anc=$5; rm -rf ch_$nm; s=$(date +%s)
  HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=/root/engine/keys-rtch taskset -c 0-15 $H libzync-prove-range $lo $hi $blk $anc ch_$nm > ch_$nm.log 2>&1; rc=$?
  echo "$nm (c-heap, 2^29) rc=$rc $(( $(date +%s)-s )) s :: $(grep -ac 're-split' ch_$nm.log) re-splits :: $(grep -a 'panicked\|block 12[67]:' ch_$nm.log | tail -2 | tr '\n' ' ' | cut -c1-220)"; }
run tap_small 126 127 $T/tap_small $T/anchor.json
run light_only 126 126 $B/light_only $B/anchor.json
echo CH_DONE
