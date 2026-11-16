#!/bin/bash
# mkbundle.sh <regtest guest program> <out dir>: the pod bundle for the zAPP regtest GPU batch (README.md "GPU batch").
# repo HEAD (the pod builds the CUDA host from it), the regtest guest program and its pins, and the block files of both
# branches (main 1..110 + a 111..112; main 1..110 + b 111..113). Spends nothing.
# The guest program: extract_guest.py from CI's libzync-hosts artefact host_reg (build-hosts.sh, the reproducible
# container); its id must be REGTEST_ID below, which the pod's build.rs recomputes from the bytes (prep.sh asserts it).
set -euo pipefail
REGTEST_ID=${REGTEST_ID:-eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd}    # CI run 43835411815, main 797b777e
REGTEST_SHA=${REGTEST_SHA:-1c3d671a6c9e95eb15d064980cd89aa5f0c41766aae5a82338c0f8416529f79b}  # extract_guest.py of that host_reg
G=$(readlink -f "${1:?regtest guest program}"); O=${2:?out dir}; HERE=$(cd "$(dirname "$0")/.." && pwd); R=$(git -C "$HERE" rev-parse --show-toplevel)
[ -z "$(git -C "$R" status --porcelain -- prover libzync patches scripts/zapp-regtest)" ] || { echo "commit first: the pod builds git HEAD"; exit 1; }
[ "$(sha256sum "$G" | cut -c1-64)" = "$REGTEST_SHA" ] || { echo "$G is not the regtest guest $REGTEST_SHA"; exit 1; }
(cd "$HERE" && sha256sum -c --quiet SHA256SUMS) || { echo "the committed fixtures do not match SHA256SUMS"; exit 1; }
B=$O/bundle; rm -rf "$B"; mkdir -p "$B/pins" "$B/json_a" "$B/json_b"
cp "$G" "$B/method.bin"
echo "$REGTEST_ID" > "$B/pins/METHOD_ID"; echo "$REGTEST_SHA" > "$B/pins/GUEST_ELF_SHA256"
echo "libzync=1 assumevalid=0 network=regtest coinlog=1 trie=1 utxohash=1" > "$B/pins/PROFILE"
echo regtest > "$B/NETWORK"; echo 1 > "$B/COINLOG"; echo 1 > "$B/TRIE"
cp "$HERE"/blocks/main/block_*.json "$B/json_a/"; cp "$HERE"/blocks/main/block_*.json "$B/json_b/"
cp "$HERE"/blocks/a/block_*.json "$B/json_a/"; cp "$HERE"/blocks/b/block_*.json "$B/json_b/"
[ "$(ls "$B/json_a" | wc -l)" = 112 ] && [ "$(ls "$B/json_b" | wc -l)" = 113 ] || { echo "block files: want 112 + 113"; exit 1; }
git -C "$R" archive --format=tar.gz -o "$B/repo.tgz" HEAD
cd "$B" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
mkdir -p "$O/sb" && cd "$O/sb" && tar czf sb.tgz -C "$B" . && sha256sum sb.tgz > sb.tgz.sha256
echo "BUNDLE_OK image $REGTEST_ID commit $(git -C "$R" rev-parse --short HEAD) $(du -h sb.tgz | cut -f1)"
