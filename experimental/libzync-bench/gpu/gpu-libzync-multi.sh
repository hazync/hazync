#!/usr/bin/env bash
# libzync GPU run B (COINLOG_GPU_PLAN.md), the PRODUCT image (coin log + counting trie + live-coin hash): prove block
# 962,000 on ONE box with N GPUs, chunks AND aggregate spread. A DEMO proof: the coin log and trie around the block are
# MODELLED (lz::cl_synth_around, from the block's own spends and $BUNDLE/txcounts.txt), so the cycles are a real block's
# and the state is not history. The trie update (mode 17) is PROVED, as in a chain.
#
#   BUNDLE=/workspace/libzync-bundle ./experimental/libzync-bench/gpu/gpu-libzync-multi.sh
#
# Bundle: method.bin (the prebuilt product guest), pins/ (METHOD_ID, GUEST_ELF_SHA256, PROFILE), block_962000.json,
# txcounts.txt. Optional EXPECT_CYCLES: the aggregate's "block validation cycles" from a laptop execute of the same
# image and bundle; the pod must reproduce it before proving anything.
#   1. build the CUDA host around the prebuilt guest; METHOD_ID must equal the pin;
#   2. the hint bundle (libzync-hints: chunk signature hints, the aggregate's ECMH hints), then an EXECUTE check of the
#      whole block (agg-exec-fake): no REJECT, and EXPECT_CYCLES when given;
#   3. the 16 chunks in N lanes, lane k on GPU k (CUDA_VISIBLE_DEVICES), chunks i with i % N == k;
#   4. the aggregate on ONE card (agg-chunks; it proves the trie update first), the control;
#   5. the aggregate SPREAD: seg-serve on loopback (segments + joins handed out), N seg-connect workers, one per GPU.
# Block time = step 3 wall + step 5 wall, both measured. Harvest $OUT before releasing the pod.
set -uo pipefail
H=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
P="$H/prover"; BIN="$P/target/release/host"
BUNDLE="${BUNDLE:?set BUNDLE to the unpacked libzync bundle}"
OUT="${OUT:-$HOME/libzync-multi-$(date +%Y%m%d-%H%M)}"; mkdir -p "$OUT/receipts"
N=$(nvidia-smi -L | grep -c '^GPU')
WANT_ID=$(grep -v '^#' "$BUNDLE/pins/METHOD_ID" | head -1)
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }
echo "$WANT_ID" | grep -Eq '^[0-9a-f]{64}$' || die "pins/METHOD_ID is not a 64-hex id"
for f in method.bin pins/PROFILE pins/GUEST_ELF_SHA256 block_962000.json txcounts.txt; do [ -f "$BUNDLE/$f" ] || die "bundle has no $f"; done
[ "$(sha256sum "$BUNDLE/method.bin" | cut -c1-64)" = "$(grep -v '^#' "$BUNDLE/pins/GUEST_ELF_SHA256" | head -1)" ] || die "method.bin is not the pinned ELF"
say "CARDS: $N x $(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)"
nvidia-smi >"$OUT/nvidia-smi.txt" 2>&1

# Only the levers set here reach the build (an inherited HAZYNC_* would change the image while the pins still pass).
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  say "=== 1. build ==="; T0=$SECONDS
  ( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" HAZYNC_GUEST_PIN_DIR="$BUNDLE/pins" HAZYNC_LIBZYNC=1 \
      cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1
  echo "REAL_EXIT=$?" >>"$OUT/build.log"
  grep -q "REAL_EXIT=0" "$OUT/build.log" || die "build failed, see $OUT/build.log"
  say "built in $((SECONDS-T0)) s"
fi
ldd "$BIN" 2>/dev/null | grep -i cuda >/dev/null || die "host is NOT linked against CUDA"
"$BIN" method-id 2>/dev/null | grep -w "$WANT_ID" >/dev/null || die "METHOD_ID is not the pinned $WANT_ID"

export HAZYNC_BLOCK="$BUNDLE/block_962000.json" HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=16 \
       HAZYNC_COINLOG_TXCOUNTS="$BUNDLE/txcounts.txt" HAZYNC_HINT_THREADS="${HAZYNC_HINT_THREADS:-$(nproc)}"

say "=== 2. hints + execute check ==="; T0=$SECONDS
( cd "$P" && "$BIN" libzync-hints "$OUT/b.lzb" ) >"$OUT/hints.log" 2>&1 || die "libzync-hints failed, see $OUT/hints.log"
export HAZYNC_LZ_HINTS="$OUT/b.lzb"
( cd "$P" && "$BIN" agg-exec-fake ) >"$OUT/exec.log" 2>&1 || die "execute check failed, see $OUT/exec.log"
grep -aq "REJECT" "$OUT/exec.log" && die "the aggregate REJECTED the block, see $OUT/exec.log"
CYC=$(grep -ao 'block validation cycles *[0-9]*' "$OUT/exec.log" | awk '{print $NF}' | tail -1)
[ -n "$CYC" ] || die "no 'block validation cycles' in $OUT/exec.log"
[ -z "${EXPECT_CYCLES:-}" ] || [ "$CYC" = "$EXPECT_CYCLES" ] || die "aggregate cycles $CYC != expected $EXPECT_CYCLES"
say "PASS: hints + execute in $((SECONDS-T0)) s, aggregate $CYC cycles${EXPECT_CYCLES:+ (= expected)}"

export HAZYNC_RECEIPTS="$OUT/receipts"
( nvidia-smi --query-gpu=index,utilization.gpu,memory.used,power.draw --format=csv,noheader -l 1 >"$OUT/smi.log" 2>/dev/null ) & SMI=$!
trap 'kill $SMI 2>/dev/null' EXIT

# ---- 3. chunks, N lanes ----
# the partition can give fewer runs than HAZYNC_CHUNKS asked for: take the count libzync-hints used
NCH=$(sed -n 's/^libzync-hints: block [0-9]* \([0-9]*\) chunks.*/\1/p' "$OUT/hints.log" | tail -1)
[ -n "$NCH" ] || die "no chunk count in $OUT/hints.log"
say "=== 3. $NCH chunks over $N GPUs ==="
lane () {
  local k=$1 i ATT RC A0
  for i in $(seq "$k" "$N" $((NCH-1))); do
    for ATT in 1 2 3 4; do
      A0=$SECONDS
      ( cd "$P" && CUDA_VISIBLE_DEVICES=$k HAZYNC_OUT="$OUT/receipts/chunk_$i.hzk" "$BIN" prove-chunk "$i" ) >"$OUT/prove_$i.log" 2>&1
      RC=$?; echo "REAL_EXIT=$RC attempt=$ATT" >>"$OUT/prove_$i.log"
      [ $RC -eq 0 ] && break
      grep -q "verification indicates proof is invalid" "$OUT/prove_$i.log" || break
    done
    echo -e "$i\t$k\t$RC\t$((SECONDS-A0))" >>"$OUT/chunks.tsv"
    [ $RC -eq 0 ] || { echo "LANE_FAIL chunk $i" >>"$OUT/run.log"; return 1; }
  done
}
: >"$OUT/chunks.tsv"; T0=$SECONDS; pids=()
for k in $(seq 0 $((N-1))); do lane "$k" & pids+=($!); done
ok=1; for p in "${pids[@]}"; do wait "$p" || ok=0; done
CHUNK_S=$((SECONDS-T0))
[ $ok = 1 ] || die "a chunk lane failed, see $OUT/run.log"
say "chunks: all $NCH proved in $CHUNK_S s wall on $N GPUs"

# ---- 4. control: the aggregate on one card ----
if [ "${SKIP_CONTROL:-0}" != "1" ]; then
  say "=== 4. aggregate on ONE card (control) ==="; T0=$SECONDS
  ( cd "$P" && CUDA_VISIBLE_DEVICES=0 HAZYNC_AGG_OUT="$OUT/block_1card.receipt" "$BIN" agg-chunks ) >"$OUT/agg_1card.log" 2>&1
  RC=$?; echo "REAL_EXIT=$RC" >>"$OUT/agg_1card.log"
  AGG1_S=$((SECONDS-T0)); [ $RC -eq 0 ] || die "one-card aggregate failed, see $OUT/agg_1card.log"
  say "one-card aggregate (trie update proved first) proved + VERIFIED in $AGG1_S s"
fi

# ---- 5. the aggregate spread over N GPUs ----
say "=== 5. aggregate SPREAD: seg-serve + $N local workers ==="; T0=$SECONDS
( cd "$P" && CUDA_VISIBLE_DEVICES=0 HAZYNC_AGG=1 HAZYNC_OUT="$OUT/block_spread.receipt" "$BIN" seg-serve ) >"$OUT/agg_spread.log" 2>&1 & SRV=$!
for t in $(seq 1 600); do grep -q "listening on" "$OUT/agg_spread.log" && break; kill -0 $SRV 2>/dev/null || break; sleep 0.5; done
wpids=()
for k in $(seq 0 $((N-1))); do
  ( cd "$P" && CUDA_VISIBLE_DEVICES=$k HAZYNC_WORKER_ID="gpu$k" "$BIN" seg-connect 127.0.0.1:${HAZYNC_PORT:-9110} ) >"$OUT/worker_$k.log" 2>&1 & wpids+=($!)
done
wait $SRV; RC=$?; echo "REAL_EXIT=$RC" >>"$OUT/agg_spread.log"
AGGN_S=$((SECONDS-T0))
sleep 2; for p in "${wpids[@]}"; do kill "$p" 2>/dev/null; done
[ $RC -eq 0 ] || die "spread aggregate failed, see $OUT/agg_spread.log"
grep -q "PUSH-TRANSPORT RECEIPT VERIFIED" "$OUT/agg_spread.log" || die "spread aggregate did not verify"
say "spread aggregate proved + VERIFIED in $AGGN_S s"
grep -E '^  (prologue|segment phase|    of which|assembly|TOTAL|serial floor)' "$OUT/agg_spread.log" | tee -a "$OUT/run.log"

say "SUMMARY: chunks $CHUNK_S s on $N GPUs + aggregate ${AGGN_S} s spread (one card: ${AGG1_S:-skipped} s) = block $((CHUNK_S+AGGN_S)) s"
say "DONE. Harvest $OUT BEFORE releasing the pod."
