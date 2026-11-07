#!/bin/bash
# A REAL chain proof from genesis with the PUBLISHED image (user 11-07: the website zAPP page needs one): blocks
# LO..HI as one recursive chain (block N's aggregate verifies N-1's receipt), coin log and trie in memory, then the
# final receipt verified with libzync-verify. Keeps what a fact server needs: the final receipt and state, the coin
# log and trie at HI, history.bin, times.tsv.
#   BUNDLE=<dir: method.bin pins/ blocks/block_<h>.json> LO=1 HI=200 OUT=<dir> bash gpu-range.sh
set -uo pipefail
P=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../prover" && pwd)
BUNDLE="${BUNDLE:?}"; LO=${LO:-1}; HI=${HI:?}; OUT="${OUT:?}"; mkdir -p "$OUT"
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done; unset RISC0_DEV_MODE
WANT_ID=$(grep -v '^#' "$BUNDLE/pins/METHOD_ID" | head -1)
[ "$(sha256sum "$BUNDLE/method.bin" | cut -c1-64)" = "$(grep -v '^#' "$BUNDLE/pins/GUEST_ELF_SHA256" | head -1)" ] || die "method.bin is not the pinned ELF"
say "CARD: $(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader | head -1); range $LO..$HI; image $WANT_ID"
say "=== build libzync host (cuda, the pinned guest) ==="
( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" HAZYNC_GUEST_PIN_DIR="$BUNDLE/pins" HAZYNC_LIBZYNC=1 \
    cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1 || die "build failed, see $OUT/build.log"
LZ="$P/target/release/host"
"$LZ" method-id | grep -w "$WANT_ID" >/dev/null || die "METHOD_ID is not the pinned $WANT_ID"
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 5 >"$OUT/smi.log" 2>/dev/null ) & SMI=$!
trap 'kill $SMI 2>/dev/null' EXIT
say "=== prove $LO..$HI from genesis ==="
t=$(date +%s)
( cd "$P" && HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=$(nproc) \
    "$LZ" libzync-prove-range "$LO" "$HI" "$BUNDLE/blocks" genesis "$OUT/run" ) > "$OUT/prove.log" 2>&1; rc=$?
say "prove-range exit $rc in $(( $(date +%s) - t )) s: $(grep -a 'LZ_PROVE_RANGE_DONE' "$OUT/prove.log" | cut -c1-120)"
[ "$rc" = 0 ] || { tail -20 "$OUT/prove.log"; die "prove-range failed"; }
say "=== verify the final receipt (libzync-verify, real, against this image) ==="
"$LZ" libzync-verify "$OUT/run/block_$HI.rcpt" > "$OUT/verify.log" 2>&1; vrc=$?
say "libzync-verify exit $vrc: $(grep -a 'LIBZYNC_VERIFY' "$OUT/verify.log" | head -2 | tr '\n' ' ')"
# keep the final receipt, the states, the coin log / trie at HI, history; drop the per-block receipts but the last
mkdir -p "$OUT/keep"; cp "$OUT/run/block_$HI.rcpt" "$OUT/run/state_$HI.json" "$OUT/run/history.bin" "$OUT/run/times.tsv" "$OUT/keep/" 2>/dev/null
cp "$OUT/run/state_$HI.coinlog" "$OUT/run/state_$HI.trie" "$OUT/keep/" 2>/dev/null
cp "$OUT/run"/state_*.json "$OUT/keep/" 2>/dev/null
say "RANGE_RUN_DONE $(ls "$OUT/keep" | wc -l) files kept"
