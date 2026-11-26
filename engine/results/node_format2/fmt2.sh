#!/bin/bash
# Node format 2 end to end (6fd07c19+): the product host + its RISC Zero guest and the engine guest built from the same
# commit, block 976,379 exported (RISC Zero's journals now in format 2), every chunk and the trie executed on the engine
# against them, the aggregate on a metering build; the trie's cost against format 1's (keys-m26 era, 3.34B / 6.31B).
source ~/.cargo/env; export PATH=$HOME/.cargo/bin:$HOME/.risc0/bin:$PATH
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine-kit && git fetch -q origin engine/openvm && git merge -q --ff-only FETCH_HEAD && git log --oneline -1 | cut -c1-60
(cd engine/host && nice cargo build --release 2>&1 | tail -1)
E=/root/engine-kit/engine/host/target/release/hzengine
# 1) product host from source (RISC Zero guest rebuilt: libzync changed)
cd /root/engine-src && git fetch -q origin engine/openvm && git checkout -q -B engine/openvm FETCH_HEAD && git log --oneline -1 | cut -c1-60
cd prover && s=$(date +%s)
HAZYNC_BASE=/root/product-base HAZYNC_LIBZYNC=1 nice cargo build --release -p host > /root/engine/fmt2_host.log 2>&1 || { echo HOST_BUILD_FAIL; grep -a "^error" -A6 /root/engine/fmt2_host.log | head; exit 1; }
echo "host $(( $(date +%s)-s )) s"; H=/root/engine-src/prover/target/release/host; $H method-id 2>&1 | head -1
# 2) engine guest (mainnet, 2^29: the VM keys of keys-rt29) assembled at HEAD
cd /root/engine || exit 1
rm -rf /root/engine-guestf2 && /root/engine-kit/engine/guest/assemble.sh /root/engine-guestf2 > asmf2.log 2>&1 || { echo ASM_FAIL; tail -5 asmf2.log; exit 1; }
rm -rf keys-f2 && mkdir keys-f2 && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-f2/
HZ_BASE=/root/engine-base5 nice $E build --dir keys-f2 --guest /root/engine-guestf2/prover/methods/guest > f2_build.log 2>&1 || { echo BUILD_FAIL; grep -a "^error\|panicked" -A6 f2_build.log | head; exit 1; }
echo "engine guest: $(grep -a '^build:' f2_build.log | cut -c1-120)"
rm -rf keys-f2b && mkdir keys-f2b && cp keys-rt29/{sdk.pk,agg.vk,openvm.toml} keys-f2b/
HZ_GUEST_FEATURES=bench-noverify HZ_BASE=/root/engine-base5 nice $E build --dir keys-f2b --guest /root/engine-guestf2/prover/methods/guest > f2b_build.log 2>&1 || { echo B_BUILD_FAIL; exit 1; }
# 3) export 976,379
X=/root/engine/xf2/976379t; rm -rf $X; mkdir -p /root/engine/xf2
export HAZYNC_BLOCK=/root/fresh-bench/block_976379.json HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 HAZYNC_CHUNK_TARGET=900000000 \
       HAZYNC_COINLOG_TXCOUNTS=/root/fresh-bench/hz_txcounts.txt HAZYNC_HINT_THREADS=16
cd /root/engine-src/prover && $H libzync-hints /root/engine/xf2/b.lzb > /root/engine/xf2/hints.log 2>&1 && HAZYNC_LZ_HINTS=/root/engine/xf2/b.lzb $H engine-export $X > /root/engine/xf2/export.log 2>&1 || { echo EXPORT_FAIL; grep -a panicked -A2 /root/engine/xf2/*.log | head; exit 1; }
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
cd /root/engine || exit 1
# 4) engine vs RISC Zero, journal by journal
for f in $X/chunk_*.in $X/trie.in; do b=${f%.in}
  r=$(taskset -c 0-15 $E execute --dir keys-f2 --input $f --journal $b.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute\|rror" | cut -c1-90 | head -1)
  cmp -s $b.j $b.r0journal && m=IDENTICAL || m=DIFFERENT; echo "$(basename $f) $m :: $r"; done
a=$(taskset -c 0-15 $E execute --dir keys-f2b --input $X/agg.in --self-id --journal $X/agg.j 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -a "^execute:\|rror\|panicked" | head -1 | cut -c1-90)
echo "aggregate: $(cmp -l $X/agg.j $X/agg.r0journal | wc -l) bytes differ of $(stat -c %s $X/agg.j) :: $a"
python3 -c "
import struct; b=open('$X/agg.j','rb').read(); w=struct.unpack('<%dI'%(len(b)//4), b)
print('profile word candidates with bit 7:', [hex(x) for x in w[:40] if x & 0x80 and x < 0x200])"
echo FMT2_DONE
