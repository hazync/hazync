#!/bin/bash
# Signet re-prove bundle (round-4 image: ChainState max_time + profile): repo HEAD, the signet guest + pins,
# blocks 2501..2600, the anchor = the laptop sweep's committed state at 2500 (same image), the 2600 hints file.
S=~/libzync-gpu/signet; B=$S/bundle2; R4=~/libzync-gpu/r18/r4/sig; rm -rf $B; mkdir -p $B/json $B/pins
cp ~/dev/hazync-libzync/prover/target/riscv-guest/methods/method/riscv32im-risc0-zkvm-elf/release/method.bin $B/
grep -ao 'METHOD_ID [0-9a-f]*' ~/libzync-gpu/r18/build_signet.log | tail -1 | awk '{print $2}' > $B/pins/METHOD_ID
# The id comes from the signet build's log and method.bin from prover/target: refuse an empty id. The pod's host
# embeds method.bin and prints ITS id, so prep.sh's exact match catches a bin from another build (review 5).
grep -Eq '^[0-9a-f]{64}$' $B/pins/METHOD_ID || { echo "no METHOD_ID in build_signet.log"; exit 1; }
sha256sum $B/method.bin | cut -c1-64 > $B/pins/GUEST_ELF_SHA256
echo "libzync=1 assumevalid=0 network=signet" > $B/pins/PROFILE; echo signet > $B/NETWORK
for h in $(seq 2501 2600); do cp $S/json/block_$h.json $B/json/; done
cp $R4/state_2500.json $B/anchor.json
cp ~/dev/hazync-libzync/experimental/libzync-bench/signet/signet2600.hints $B/signet.hints
git -C ~/dev/hazync-libzync archive --format=tar.gz -o $B/repo.tgz HEAD
cd $B && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
mkdir -p $S/sb2 && cd $S/sb2 && tar czf sb.tgz -C $B . && sha256sum sb.tgz > sb.tgz.sha256 && ls -la sb.tgz && cat $B/pins/METHOD_ID && git -C ~/dev/hazync-libzync log --oneline -1
