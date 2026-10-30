#!/bin/bash
# Pack the signet GPU bundle: repo snapshot (HEAD of libzync), prebuilt signet guest + pins, blocks LO..HI,
# the anchor state at LO-1 (from the laptop sweep against the same hints file), and that hints file.
LO=$1; HI=$2; H=$3   # e.g. 2501 2600 2600
S=~/libzync-gpu/signet; B=$S/bundle; R=~/libzync-gpu/r18/signet
rm -rf $B/json $B/repo.tgz $B/anchor.json $B/signet.hints; mkdir -p $B/json
for h in $(seq $LO $HI); do cp $S/json/block_$h.json $B/json/; done
cp $R/states$H/state_$((LO-1)).json $B/anchor.json
cp $R/signet$H.hints $B/signet.hints
git -C ~/dev/hazync-libzync archive --format=tar.gz -o $B/repo.tgz HEAD
cd $B && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
cd $S && tar czf sb.tgz -C $B . && sha256sum sb.tgz > sb.tgz.sha256 && ls -la sb.tgz && git -C ~/dev/hazync-libzync log --oneline -1
