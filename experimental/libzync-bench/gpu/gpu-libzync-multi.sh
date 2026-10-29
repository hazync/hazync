#!/usr/bin/env bash
# libzync GPU test, multi-card: prove block 962,000 on ONE box with N GPUs, chunks AND aggregate spread.
#
#   BUNDLE=/workspace/libzync-bundle ./experimental/libzync-bench/gpu/gpu-libzync-multi.sh
#
# Same build, pins and execute check as gpu-libzync.sh (run that first or set SKIP_BUILD=1 after it).
# Then:
#   3. the 16 chunks in N lanes, lane k on GPU k (CUDA_VISIBLE_DEVICES), chunks i with i % N == k;
#   4. the aggregate on ONE card (agg-chunks), the control: same box, same receipts;
#   5. the aggregate SPREAD: seg-serve on loopback (segments + joins handed out), N seg-connect workers,
#      one per GPU. Loopback means no RunPod hairpin problem (memory: gotcha_runpod_no_hairpin_to_public_port).
# Block time = step 3 wall + step 5 wall, both measured.
set -uo pipefail
H=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
P="$H/prover"; BIN="$P/target/release/host"
BUNDLE="${BUNDLE:?set BUNDLE to the unpacked libzync bundle}"
OUT="${OUT:-$HOME/libzync-multi-$(date +%Y%m%d-%H%M)}"; mkdir -p "$OUT/receipts"
N=$(nvidia-smi -L | grep -c '^GPU')
WANT_ID=$(grep -v '^#' "$H/reproduce/METHOD_ID" | grep -m1 .)
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }
say "CARDS: $N x $(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)"
nvidia-smi >"$OUT/nvidia-smi.txt" 2>&1

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  say "=== 1. build ==="; T0=$SECONDS
  ( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1
  echo "REAL_EXIT=$?" >>"$OUT/build.log"
  grep -q "REAL_EXIT=0" "$OUT/build.log" || die "build failed, see $OUT/build.log"
  say "built in $((SECONDS-T0)) s"
fi
ldd "$BIN" 2>/dev/null | grep -qi cuda || die "host is NOT linked against CUDA"
GOT_ID=$("$BIN" method-id | awk '/METHOD_ID/{print $2}')
[ "$GOT_ID" = "$WANT_ID" ] || die "METHOD_ID $GOT_ID != pinned $WANT_ID"

export HAZYNC_BLOCK="$BUNDLE/block_962000.json" HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost \
       HAZYNC_BATCH_ECDSA=1 HAZYNC_SWIFTSYNC=1 HAZYNC_AGG_READSLICE=1 HAZYNC_AGG_LITE=1 \
       HAZYNC_ECMH_HINTS="$BUNDLE/ecmh_g.bin"
# shellcheck disable=SC2046
export $(sed 's/^FIT=//' "$BUNDLE/fit_env.txt")
export HAZYNC_COST_MODEL=fit HAZYNC_PACK_KEYS=1 HAZYNC_COST_EC_OP=14537 HAZYNC_COST_EC_OP_REPEAT=0

say "=== 2. execute check ==="
( cd "$P" && HAZYNC_CHUNKS=1 HAZYNC_PROFILE_EXEC=1 HAZYNC_BATCH_HINTS_DIR="$BUNDLE/h1" "$BIN" chunk-profile ) >"$OUT/exec1.log" 2>&1
grep -q 'journal sha256 98e2652d5184e30a' "$OUT/exec1.log" || die "journal mismatch, see $OUT/exec1.log"
grep -qE 'cycles +326652314' "$OUT/exec1.log" || die "cycle mismatch, see $OUT/exec1.log"
say "PASS: journal 98e2652d, cycles 326652314"

export HAZYNC_CHUNKS=16 HAZYNC_BATCH_HINTS_DIR="$BUNDLE/hg" HAZYNC_RECEIPTS="$OUT/receipts"
( nvidia-smi --query-gpu=index,utilization.gpu,memory.used,power.draw --format=csv,noheader -l 1 >"$OUT/smi.log" 2>/dev/null ) & SMI=$!
trap 'kill $SMI 2>/dev/null' EXIT

# ---- 3. chunks, N lanes ----
say "=== 3. 16 chunks over $N GPUs ==="
lane () {
  local k=$1 i ATT RC A0
  for i in $(seq "$k" "$N" 15); do
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
say "chunks: all 16 proved in $CHUNK_S s wall on $N GPUs"

# ---- 4. control: the aggregate on one card ----
if [ "${SKIP_CONTROL:-0}" != "1" ]; then
  say "=== 4. aggregate on ONE card (control) ==="; T0=$SECONDS
  ( cd "$P" && CUDA_VISIBLE_DEVICES=0 HAZYNC_AGG_OUT="$OUT/block_1card.receipt" "$BIN" agg-chunks ) >"$OUT/agg_1card.log" 2>&1
  RC=$?; echo "REAL_EXIT=$RC" >>"$OUT/agg_1card.log"
  AGG1_S=$((SECONDS-T0)); [ $RC -eq 0 ] || die "one-card aggregate failed"
  say "one-card aggregate proved + VERIFIED in $AGG1_S s"
fi

# ---- 5. the aggregate spread over N GPUs ----
say "=== 5. aggregate SPREAD: seg-serve + $N local workers ==="; T0=$SECONDS
( cd "$P" && CUDA_VISIBLE_DEVICES=0 HAZYNC_AGG=1 HAZYNC_OUT="$OUT/block_spread.receipt" "$BIN" seg-serve ) >"$OUT/agg_spread.log" 2>&1 & SRV=$!
for t in $(seq 1 120); do grep -q "listening on" "$OUT/agg_spread.log" && break; kill -0 $SRV 2>/dev/null || break; sleep 0.5; done
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
