#!/bin/bash
# mkbundle_multi.sh <keys> <out.tgz> <host src> <rblocks dir> <export dir>...: pod/run_multi.sh's bundle: keys/,
# export/<name>/ per block export, host/, rblocks/ (block JSONs + hz_txcounts.txt for the RISC Zero runs).
set -euo pipefail
K=$1; O=$2; H=$3; RB=$4; shift 4; T=$(mktemp -d)
mkdir -p $T/keys $T/export $T/host $T/rblocks
for f in sdk.pk agg.vk openvm.toml guest.vmexe baseline.json image.json; do cp -L "$K/$f" $T/keys/; done
for X in "$@"; do n=$(basename $X); n=${n#export}; mkdir -p $T/export/$n
  (cd $X && cp chunks method_id agg.in agg.r0journal trie.in trie.r0journal chunk_*.in chunk_*.r0journal $T/export/$n/); done
(cd "$H" && tar cf - --exclude=target .) | (cd $T/host && tar xf -)
cp -r "$RB"/. $T/rblocks/
tar czf "$O" -C $T keys export host rblocks; rm -rf $T; sha256sum "$O"; du -h "$O"
