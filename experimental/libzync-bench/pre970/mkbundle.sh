#!/bin/bash
# Mainnet pre-H bundle: repo snapshot, the mainnet libzync guest + pins, blocks 969,996..970,000, the anchor state at
# 969,995 (derived from the block file: a demonstration anchor), and the published hints file (stop 970,000).
P=~/libzync-gpu/pre970; B=$P/bundle; rm -rf $B; mkdir -p $B/json $B/pins
cp ~/dev/hazync-libzync/prover/target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin $B/
grep -o 'METHOD_ID [0-9a-f]*' ~/libzync-gpu/r18/build_main.log | tail -1 | awk '{print $2}' > $B/pins/METHOD_ID
sha256sum $B/method.bin | cut -c1-64 > $B/pins/GUEST_ELF_SHA256
echo mainnet > $B/NETWORK
for h in 969996 969997 969998 969999 970000; do cp $P/json/block_$h.json $B/json/; done
HAZYNC_BLOCK=$P/json/block_969996.json ~/libzync-gpu/r18/host_main libzync-anchor-json $B/anchor.json
cp ~/libzync-gpu/hints/bitcoin.hints $B/signet.hints   # run.sh's fixed name; this is the MAINNET file
git -C ~/dev/hazync-libzync archive --format=tar.gz -o $B/repo.tgz HEAD
cd $B && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
cd $P && tar czf sb.tgz -C $B . && sha256sum sb.tgz > sb.tgz.sha256 && ls -la sb.tgz && cat $B/pins/METHOD_ID
