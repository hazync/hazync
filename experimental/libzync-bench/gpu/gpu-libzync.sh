#!/usr/bin/env bash
# libzync GPU test: prove block 962,000 with the round-13 guest on ONE card, end to end.
#
#   BUNDLE=/workspace/libzync-bundle ./experimental/libzync-bench/gpu/gpu-libzync.sh
#
# The guest is NOT built here. The bundle carries the exact method.bin measured on the laptop, and the
# host embeds it via HAZYNC_GUEST_ELF, which refuses it unless its sha256 and image id equal the pins in
# reproduce/ (set on the libzync-gpu branch). So this box needs no Core / secp trees and no riscv
# toolchain, and its cycle counts must equal the laptop's EXACTLY (checked in step 2).
#
# Guards carried over from scripts/gpu-benchmark.sh, each from a failure that looked like a result
# (memory: gotcha_gpu_harness_failures_all_look_like_results):
#   - every step writes REAL_EXIT into its own log; a background notifier's exit code is not trusted
#   - ldd must show libcuda, or this would "prove" at CPU speed
#   - METHOD_ID printed and compared to the pin
#   - #119 retry, and the chunk is timed on its SUCCESSFUL attempt only
#   - never `pgrep -f` a pattern that appears in this script's own command line
set -uo pipefail
H=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
P="$H/prover"; BIN="$P/target/release/host"
BUNDLE="${BUNDLE:?set BUNDLE to the unpacked libzync bundle}"
OUT="${OUT:-$HOME/libzync-gpu-$(date +%Y%m%d-%H%M)}"; mkdir -p "$OUT/receipts"
WANT_ID=$(grep -v '^#' "$H/reproduce/METHOD_ID" | grep -m1 .)
WANT_JOURNAL=98e2652d5184e30a1f291dcf3cfed3b145ce0bfb55540005a826c9e0fadec586
WANT_CYCLES=326652314
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }

say "CARD: $(nvidia-smi --query-gpu=name,memory.total --format=csv,noheader 2>/dev/null | head -1 || echo UNKNOWN)"
say "output: $OUT   bundle: $BUNDLE   want METHOD_ID $WANT_ID"
nvidia-smi >"$OUT/nvidia-smi.txt" 2>&1

# ---- 1. build the CUDA host around the prebuilt guest ----
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  say "=== 1. build (release + cuda, prebuilt guest) ==="
  T0=$SECONDS
  ( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1
  echo "REAL_EXIT=$?" >>"$OUT/build.log"
  grep -q "REAL_EXIT=0" "$OUT/build.log" || die "build failed, see $OUT/build.log"
  say "built in $((SECONDS-T0)) s"
fi
ldd "$BIN" 2>/dev/null | grep -qi cuda || die "host is NOT linked against CUDA"
GOT_ID=$("$BIN" method-id | awk '/METHOD_ID/{print $2}')
[ "$GOT_ID" = "$WANT_ID" ] || die "METHOD_ID $GOT_ID != pinned $WANT_ID"
grep -qa "agg-lite: REJECT chunk context" "$BIN" || die "embedded guest lacks the agg-lite strings — wrong image"
say "assert OK: CUDA linked, METHOD_ID $GOT_ID, agg-lite guest embedded"

# Runtime env: identical to the laptop's round-13 measurement (r13/run.sh). Every hint block the
# guest reads must be switched on here exactly as at build time, or the input stream desyncs.
export HAZYNC_BLOCK="$BUNDLE/block_962000.json" HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost \
       HAZYNC_BATCH_ECDSA=1 HAZYNC_SWIFTSYNC=1 HAZYNC_AGG_READSLICE=1 HAZYNC_AGG_LITE=1 \
       HAZYNC_ECMH_HINTS="$BUNDLE/ecmh_g.bin"
# shellcheck disable=SC2046
export $(sed 's/^FIT=//' "$BUNDLE/fit_env.txt")
export HAZYNC_COST_MODEL=fit HAZYNC_PACK_KEYS=1 HAZYNC_COST_EC_OP=14537 HAZYNC_COST_EC_OP_REPEAT=0

# ---- 2. execute check: same journal and the SAME cycle count as the laptop ----
say "=== 2. execute check, whole block as one chunk ==="
( cd "$P" && HAZYNC_CHUNKS=1 HAZYNC_PROFILE_EXEC=1 HAZYNC_BATCH_HINTS_DIR="$BUNDLE/h1" "$BIN" chunk-profile ) >"$OUT/exec1.log" 2>&1
echo "REAL_EXIT=$?" >>"$OUT/exec1.log"
J=$(grep -o 'journal sha256 [0-9a-f]*' "$OUT/exec1.log" | tail -1 | awk '{print $3}')
C=$(grep -oE 'cycles +[0-9]+' "$OUT/exec1.log" | tail -1 | awk '{print $2}')
[ "$J" = "$WANT_JOURNAL" ] || die "journal $J != $WANT_JOURNAL"
[ "$C" = "$WANT_CYCLES" ] || die "cycles $C != laptop $WANT_CYCLES — not the measured build"
say "PASS: journal ${J:0:8}, cycles $C (= laptop)"

# ---- 3. prove the 16 chunks, one after another on this card ----
say "=== 3. prove 16 chunks (po2 ${HAZYNC_SEG_PO2:-default}) ==="
export HAZYNC_CHUNKS=16 HAZYNC_BATCH_HINTS_DIR="$BUNDLE/hg"
: >"$OUT/chunks.tsv"
for i in $(seq 0 15); do
  ( nvidia-smi --query-gpu=memory.used,power.draw --format=csv,noheader -l 2 >"$OUT/smi_$i.log" 2>/dev/null ) & SMI=$!
  T0=$SECONDS; RC=1
  for ATT in 1 2 3 4; do
    A0=$SECONDS
    ( cd "$P" && HAZYNC_OUT="$OUT/receipts/chunk_$i.hzk" "$BIN" prove-chunk "$i" ) >"$OUT/prove_$i.log" 2>&1
    RC=$?; echo "REAL_EXIT=$RC attempt=$ATT" >>"$OUT/prove_$i.log"
    [ $RC -eq 0 ] && break
    grep -q "verification indicates proof is invalid" "$OUT/prove_$i.log" || break
    say "  [#119] chunk $i invalid receipt on attempt $ATT, retrying"
  done
  kill $SMI 2>/dev/null
  SEGS=$(grep -oE '[0-9]+ segments at po2' "$OUT/prove_$i.log" | awk '{print $1}')
  echo -e "$i\t$RC\t$((SECONDS-A0))\t$((SECONDS-T0))\t${SEGS:-?}" | tee -a "$OUT/chunks.tsv" >/dev/null
  say "chunk $i rc=$RC wall_s=$((SECONDS-A0)) total_s=$((SECONDS-T0)) segments=${SEGS:-?}"
  [ $RC -eq 0 ] || die "chunk $i failed, see $OUT/prove_$i.log"
done

# ---- 4. aggregate from the REAL receipts: execute first (stream check), then prove ----
export HAZYNC_RECEIPTS="$OUT/receipts"
say "=== 4a. aggregate, execute only, from the real chunk receipts ==="
( cd "$P" && HAZYNC_AGG_EXECUTE=1 "$BIN" agg-chunks ) >"$OUT/agg_exec.log" 2>&1
echo "REAL_EXIT=$?" >>"$OUT/agg_exec.log"
grep -q "REAL_EXIT=0" "$OUT/agg_exec.log" || die "aggregate execute failed, see $OUT/agg_exec.log"
say "aggregate execute OK: $(grep -a 'acc_x_top_word' "$OUT/agg_exec.log" | tail -1)"
say "=== 4b. aggregate, PROVING ==="
T0=$SECONDS
( cd "$P" && HAZYNC_AGG_OUT="$OUT/block_962000.receipt" "$BIN" agg-chunks ) >"$OUT/agg_prove.log" 2>&1
RC=$?; echo "REAL_EXIT=$RC" >>"$OUT/agg_prove.log"
AGG_S=$((SECONDS-T0))
[ $RC -eq 0 ] || die "aggregate proof failed, see $OUT/agg_prove.log"
say "aggregate proved + VERIFIED in $AGG_S s"

# ---- 5. summary ----
python3 - "$OUT" "$AGG_S" <<'PY' | tee -a "$OUT/run.log"
import sys
out, agg = sys.argv[1], int(sys.argv[2])
rows = [l.split("\t") for l in open(f"{out}/chunks.tsv") if l.strip()]
w = [int(r[2]) for r in rows]
print(f"chunks: sum {sum(w)} s, max {max(w)} s, mean {sum(w)/len(w):.0f} s, straggler {max(w)/(sum(w)/len(w)):.2f}")
print(f"aggregate: {agg} s")
print(f"ONE-CARD wall clock (sequential): {sum(w) + agg} s")
print(f"FLEET critical path (16 cards for chunks + 1, no overheads): {max(w) + agg} s")
print(f"card-seconds per block: {sum(w) + agg}")
PY
say "DONE. Harvest $OUT BEFORE releasing the pod (tar czf ~/libzync-gpu.tgz -C $(dirname "$OUT") $(basename "$OUT"))."
