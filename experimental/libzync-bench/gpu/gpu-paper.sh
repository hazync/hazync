#!/bin/bash
# The paper's same-card measurement (user 11-07): on ONE card, each block proved by libzync (c14, bbf82800) as a LEAF
# from prep's real-state leaf_<h>.bin, and by the original CORE guest (37987b85, the v0.22.1 CUDA release binary) from
# the original bridge's bundle_<h>.json. Records wall time, LZ_STATS (libzync), segments (CORE), GPU use.
#   BUNDLE=<dir: method.bin pins/ leaf/leaf_<h>.bin core/bundle_<h>.json> HEIGHTS="200000 300000" OUT=<dir> bash gpu-paper.sh
# CONCURRENCY=N (default 1): N blocks in flight at once on the card, for CORE and libzync alike, so one block's CPU
# phases (libzync: signature hints, the collect pass; CORE: execution, assembly) overlap another's GPU work. The
# card time per block is then the phase's wall divided by its blocks (results.tsv: one PHASE row per prover).
# MODELLED="<heights>": also libzync on MODELLED state (lz::cl_synth_around, as the 962,000 benchmark), from
# modelled/block_<h>.json + txcounts.txt: real block, cycles a real block's, state not history (labelled as such).
set -uo pipefail
P=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../prover" && pwd)
BUNDLE="${BUNDLE:?}"; HEIGHTS="${HEIGHTS:-}"; MODELLED="${MODELLED:-}"; OUT="${OUT:?}"; mkdir -p "$OUT"
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
# CORE first (its bundles are all here), then libzync: a leaf bundle may still be arriving (the laptop driver pushes
# each one as prep writes it), so a libzync block waits for its leaf up to LEAF_WAIT_S
C=${CONCURRENCY:-1}
throttle () { while [ "$(jobs -rp | grep -v "^$SMI$" | wc -l)" -ge "$C" ]; do sleep 2; done; }
t_phase=$(date +%s.%N); n_phase=0
for h in $HEIGHTS; do
  [ -f "$BUNDLE/core/bundle_$h.json" ] || { say "no CORE bundle for $h"; continue; }
  throttle; n_phase=$((n_phase+1))
  (
  say "CORE $h"; mkdir -p "$OUT/core_$h"
  t=$(date +%s.%N)
  ( cd "$OUT/core_$h" && HAZYNC_BRIDGE_OUT="$BUNDLE/core" "$CORE" prove-range-bridge "$h" ) > "$OUT/core_$h.log" 2>&1; rc=$?
  w=$(python3 -c "import time;print(f'{time.time()-$t:.1f}')")
  seg=$(grep -a -o 'executed, [0-9]* segments at po2 [0-9]*' "$OUT/core_$h.log" | head -1)
  echo -e "$h\tCORE\t$w\t$rc\t$seg" >> "$OUT/results.tsv"; say "CORE $h: ${w} s exit $rc ($seg)"
  ) &
done
for j in $(jobs -rp); do [ "$j" = "$SMI" ] || wait "$j"; done
[ "$n_phase" -gt 0 ] && { w=$(python3 -c "import time;print(f'{time.time()-$t_phase:.1f}')"); echo -e "PHASE\tCORE\t$w\t$n_phase\tconcurrency=$C" >> "$OUT/results.tsv"; say "CORE phase: $n_phase blocks in ${w} s at concurrency $C"; }
t_phase=$(date +%s.%N); n_phase=0
for h in $HEIGHTS; do
  f="$BUNDLE/leaf/leaf_$h.bin"; t0=$(date +%s)
  until [ -f "$f" ] || [ $(( $(date +%s) - t0 )) -ge ${LEAF_WAIT_S:-10800} ]; do sleep 20; done
  [ -f "$f" ] || { say "no leaf bundle for $h after ${LEAF_WAIT_S:-10800} s"; continue; }
  throttle; n_phase=$((n_phase+1))
  (
  say "libzync leaf $h"; rm -rf "$OUT/lz_$h"; mkdir -p "$OUT/lz_$h"
  t=$(date +%s.%N)
  HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=$(nproc) HAZYNC_LEAF_BUNDLE="$f" \
    "$LZ" libzync-prove-range "$h" "$h" "$OUT/lz_$h" leaf "$OUT/lz_$h/run" > "$OUT/lz_$h.log" 2>&1; rc=$?
  w=$(python3 -c "import time;print(f'{time.time()-$t:.1f}')")
  st=$(grep -a '^LZ_STATS' "$OUT/lz_$h.log" | awk '{print $2"="$4","$5}' | tr '\n' ' ')
  echo -e "$h\tlibzync\t$w\t$rc\t$st" >> "$OUT/results.tsv"; say "libzync $h: ${w} s exit $rc $st"
  ) &
done
for j in $(jobs -rp); do [ "$j" = "$SMI" ] || wait "$j"; done
[ "$n_phase" -gt 0 ] && { w=$(python3 -c "import time;print(f'{time.time()-$t_phase:.1f}')"); echo -e "PHASE\tlibzync\t$w\t$n_phase\tconcurrency=$C" >> "$OUT/results.tsv"; say "libzync phase: $n_phase blocks in ${w} s at concurrency $C"; }
for h in $MODELLED; do
  f="$BUNDLE/modelled/block_$h.json"; [ -f "$f" ] || { say "no modelled block file for $h"; continue; }
  say "libzync MODELLED $h"; d="$OUT/mod_$h"; rm -rf "$d"; mkdir -p "$d/receipts"
  t=$(date +%s.%N)
  ( cd "$P" && export HAZYNC_BLOCK="$f" HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_TXCOUNTS="$BUNDLE/txcounts.txt" \
        HAZYNC_HINT_THREADS=$(nproc) HAZYNC_RECEIPTS="$d/receipts" && \
    "$LZ" libzync-hints "$d/b.lzb" > "$d/hints.log" 2>&1 && export HAZYNC_LZ_HINTS="$d/b.lzb" && \
    n=$(sed -n 's/^libzync-hints: block [0-9]* \([0-9]*\) chunks.*/\1/p' "$d/hints.log" | tail -1) && \
    for i in $(seq 0 $(( ${n:-1} - 1 ))); do HAZYNC_OUT="$d/receipts/chunk_$i.hzk" "$LZ" prove-chunk "$i" >> "$d/chunks.log" 2>&1 || exit 7; done && \
    HAZYNC_AGG_OUT="$d/block.receipt" "$LZ" agg-chunks > "$d/agg.log" 2>&1 ); rc=$?
  w=$(python3 -c "import time;print(f'{time.time()-$t:.1f}')")
  ls "$d/block.receipt" >/dev/null 2>&1 || rc=${rc/#0/9}
  echo -e "$h\tlibzync-modelled\t$w\t$rc\t$(tail -c 300 "$d/agg.log" | tr '\n' ' ' | tail -c 160)" >> "$OUT/results.tsv"; say "libzync MODELLED $h: ${w} s exit $rc"
done
say "PAPER_RUN_DONE"; cat "$OUT/results.tsv" | tee -a "$OUT/run.log"
