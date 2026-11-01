#!/bin/bash
# mkbundle.sh [HI]: the signet genesis bundle. repo HEAD, the guest ELF taken from the reproducible container and
# checked against reproduce/LIBZYNC_IDS, blocks 1..HI, the hints file for H=HI. Build the container first:
#   docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .
set -euo pipefail
HI=${1:-2600}; R=$(git rev-parse --show-toplevel); S=~/libzync-gpu/signet-genesis; B=$S/bundle
J=~/libzync-gpu/signet/json; HINTS=$R/experimental/libzync-bench/signet/signet$HI.hints
[ -f "$HINTS" ] || { echo "no hints file for H=$HI: $HINTS"; exit 1; }
[ -z "$(git -C "$R" status --porcelain -- prover libzync patches scripts/libzync-base.sh reproduce)" ] || { echo "commit the prover first: the pod builds git HEAD"; exit 1; }
rm -rf "$B"; mkdir -p "$B/json" "$B/pins"
C=$(docker create hazync-libzync-repro); trap 'docker rm -f "$C" >/dev/null' EXIT
docker cp -q "$C:/root/libzync-elf/signet.bin" "$B/method.bin"
read -r M E < <(awk '$2=="signet"{print $4, $6}' "$R/reproduce/LIBZYNC_IDS")
[ "$(sha256sum "$B/method.bin" | cut -c1-64)" = "$E" ] || { echo "container ELF is not the published signet ELF"; exit 1; }
echo "$M" > "$B/pins/METHOD_ID"; echo "$E" > "$B/pins/GUEST_ELF_SHA256"
echo "libzync=1 assumevalid=0 network=signet" > "$B/pins/PROFILE"; echo signet > "$B/NETWORK"
for h in $(seq 1 "$HI"); do cp "$J/block_$h.json" "$B/json/"; done
cp "$HINTS" "$B/signet.hints"
git -C "$R" archive --format=tar.gz -o "$B/repo.tgz" HEAD
cd "$B" && find . -type f ! -name SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS
mkdir -p "$S/sb" && cd "$S/sb" && tar czf sb.tgz -C "$B" . && sha256sum sb.tgz > sb.tgz.sha256
echo "BUNDLE_OK H=$HI image $M commit $(git -C "$R" rev-parse --short HEAD) $(du -h sb.tgz | cut -f1)"
