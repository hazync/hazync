#!/bin/bash
# mkbundle.sh <keys dir> <export dir> <host src dir> <out.tgz>: what pod/setup.sh + pod/run.sh need (host source
# without target/, the key dir with real files, the block export).
set -euo pipefail
K=$1; X=$2; H=$3; O=$4; T=$(mktemp -d)
mkdir -p $T/keys $T/export $T/host
for f in sdk.pk agg.vk openvm.toml guest.vmexe baseline.json; do cp -L "$K/$f" $T/keys/; done
cp -r "$X"/. $T/export/; rm -f $T/export/*.exec.log
(cd "$H" && tar cf - --exclude=target .) | (cd $T/host && tar xf -)
tar czf "$O" -C $T keys export host; rm -rf $T
sha256sum "$O"; du -h "$O"
