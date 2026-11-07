#!/bin/bash
# The paper's same-card measurement (user 11-07): on ONE card, each block proved by libzync (c14, bbf82800) as a LEAF
# from prep's real-state leaf_<h>.bin, and by the original CORE guest (37987b85, the v0.22.1 CUDA release binary) from
# the original bridge's bundle_<h>.json. Records wall time, LZ_STATS (libzync), segments (CORE), GPU use.
#   BUNDLE=<dir: method.bin pins/ leaf/leaf_<h>.bin core/bundle_<h>.json> HEIGHTS="200000 300000" OUT=<dir> bash gpu-paper.sh
set -uo pipefail
P=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../prover" && pwd)
BUNDLE="${BUNDLE:?}"; HEIGHTS="${HEIGHTS:?}"; OUT="${OUT:?}"; mkdir -p "$OUT"
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done; unset RISC0_DEV_MODE
WANT_ID=$(grep -v '^#' "$BUNDLE/pins/METHOD_ID" | head -1)
[ "$(sha256sum "$BUNDLE/method.bin" | cut -c1-64)" = "$(grep -v '^#' "$BUNDLE/pins/GUEST_ELF_SHA256" | head -1)" ] || die "method.bin is not the pinned ELF"
say "CARD: $(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader | head -1)"
say "=== build libzync host (cuda, the pinned guest) ==="
( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" HAZYNC_GUEST_PIN_DIR="$BUNDLE/pins" HAZYNC_LIBZYNC=1 \
    cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1 || die "build failed, see $OUT/build.log"
LZ="$P/target/release/host"
"$LZ" method-id | grep -w "$WANT_ID" >/dev/null || die "libzync METHOD_ID is not the pinned $WANT_ID"
say "=== CORE: the v0.22.1 CUDA release binary ==="
CORE=/workspace/core/hazync-host-x86_64-linux-gnu-cuda; mkdir -p /workspace/core
( cd /workspace/core && curl -fsSL -O https://github.com/hazync/hazync/releases/download/v0.22.1/hazync-host-x86_64-linux-gnu-cuda \
    && curl -fsSL -O https://github.com/hazync/hazync/releases/download/v0.22.1/SHA256SUMS.txt \
    && grep ' hazync-host-x86_64-linux-gnu-cuda$' SHA256SUMS.txt | sha256sum -c - ) >>"$OUT/run.log" 2>&1 || die "CORE download / checksum failed"
chmod +x "$CORE"
"$CORE" method-id | grep -w 37987b85ec665970ac6c5e8031deb8160ac8ed846f09056c3790b5f78c8bb5dd >/dev/null || die "CORE METHOD_ID is not 37987b85"
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 1 >"$OUT/smi.log" 2>/dev/null ) & SMI=$!
trap 'kill $SMI 2>/dev/null' EXIT
echo -e "height\twhat\twall_s\texit\tdetail" > "$OUT/results.tsv"
for h in $HEIGHTS; do
  # libzync: one chunk per block (as the cycle table), signature hints collected first
  if [ -f "$BUNDLE/leaf/leaf_$h.bin" ]; then
    say "libzync leaf $h"; rm -rf "$OUT/lz_$h"; mkdir -p "$OUT/lz_$h"
    t=$(date +%s.%N)
    HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=$(nproc) HAZYNC_LEAF_BUNDLE="$BUNDLE/leaf/leaf_$h.bin" \
      "$LZ" libzync-prove-range "$h" "$h" "$OUT/lz_$h" leaf "$OUT/lz_$h/run" > "$OUT/lz_$h.log" 2>&1; rc=$?
    w=$(python3 -c "import time;print(f'{time.time()-$t:.1f}')")
    st=$(grep -a '^LZ_STATS' "$OUT/lz_$h.log" | awk '{print $2"="$4","$5}' | tr '\n' ' ')
    echo -e "$h\tlibzync\t$w\t$rc\t$st" >> "$OUT/results.tsv"; say "libzync $h: ${w} s exit $rc $st"
  fi
  if [ -f "$BUNDLE/core/bundle_$h.json" ]; then
    say "CORE $h"; mkdir -p "$OUT/core_$h"
    t=$(date +%s.%N)
    ( cd "$OUT/core_$h" && HAZYNC_BRIDGE_OUT="$BUNDLE/core" "$CORE" prove-range-bridge "$h" ) > "$OUT/core_$h.log" 2>&1; rc=$?
    w=$(python3 -c "import time;print(f'{time.time()-$t:.1f}')")
    seg=$(grep -a -o 'executed, [0-9]* segments at po2 [0-9]*' "$OUT/core_$h.log" | head -1)
    echo -e "$h\tCORE\t$w\t$rc\t$seg" >> "$OUT/results.tsv"; say "CORE $h: ${w} s exit $rc ($seg)"
  fi
done
say "PAPER_RUN_DONE"; cat "$OUT/results.tsv" | tee -a "$OUT/run.log"
