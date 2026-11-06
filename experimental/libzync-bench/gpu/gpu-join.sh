#!/usr/bin/env bash
# ROADMAP H-B (d): the REAL (non-dev) join tree on GPUs. The dev-mode test (join/test_join.sh) shows the root journal is
# byte-identical to the linear chain's; only real receipts exercise env::verify / assumption resolution and show a join
# is succinct and joinable again. Here: blocks 1..N proved as independent leaves (each a base step on the linear run's
# state before it), a balanced join tree over them (levels >= 2 join JOINS), every receipt verified against the pinned
# image, and the root's journal compared with the linear run's block_N journal. Measures leaf and join cycles / seconds.
#
#   BUNDLE=/workspace/join-bundle N=8 ./experimental/libzync-bench/gpu/gpu-join.sh
#
# Bundle (make it on the laptop with `gpu-join.sh --mkbundle <linear run dir> <block json dir> <out> [N]`):
#   method.bin, pins/ (METHOD_ID, GUEST_ELF_SHA256, PROFILE), blocks/block_<h>.json (1..N),
#   anchors/state_<h>.{json,coinlog,trie} (0..N-1; 0 = genesis is not needed), LINEAR_SHA (block_N's journal sha256).
# Leaves and each level's joins run one per GPU in parallel. Harvest $OUT before releasing the pod.
set -uo pipefail
H=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
P="$H/prover"; BIN="$P/target/release/host"

if [ "${1:-}" = "--mkbundle" ]; then
  RUN=${2:?linear run dir}; JS=${3:?block json dir}; B=${4:?out}; N=${5:-8}; IMG=${IMG:?IMG=<method.bin of the run image>}
  rm -rf "$B"; mkdir -p "$B/blocks" "$B/anchors" "$B/pins"
  cp "$IMG" "$B/method.bin"; sha256sum "$B/method.bin" | cut -c1-64 > "$B/pins/GUEST_ELF_SHA256"
  for f in METHOD_ID PROFILE; do cp "${PINS:?PINS=<pins dir of the image>}/$f" "$B/pins/$f"; done
  for h in $(seq 1 "$N"); do cp "$JS/block_$h.json" "$B/blocks/"; done
  for h in $(seq 1 $((N - 1))); do for x in json coinlog trie; do cp "$RUN/state_$h.$x" "$B/anchors/"; done; done
  # the linear run is a laptop (dev-mode) run: only its journal is taken, the bytes a real proof of the chain commits
  RISC0_DEV_MODE=1 "${HOSTBIN:?HOSTBIN=<a host of the same image>}" journal-sha "$RUN/block_$N.rcpt" | awk '/JOURNAL_SHA256/{print $2}' > "$B/LINEAR_SHA"
  [ -n "$(cat "$B/LINEAR_SHA")" ] || { echo "no journal sha for $RUN/block_$N.rcpt"; exit 1; }
  echo "bundle $B: N=$N, $(du -sh "$B" | cut -f1)"; exit 0
fi

BUNDLE="${BUNDLE:?set BUNDLE to the unpacked join bundle}"; N=${N:-8}
OUT="${OUT:-$HOME/libzync-join-$(date +%Y%m%d-%H%M)}"; mkdir -p "$OUT/leaf" "$OUT/join"
# DRY=1 DRYBIN=<host>: the script's logic on a laptop (dev mode, no GPU, no build)
if [ "${DRY:-0}" = 1 ]; then G=${G:-2}; BIN=${DRYBIN:?DRYBIN=<host binary of the bundle image>}; export RISC0_DEV_MODE=1 SKIP_BUILD=1
else G=$(nvidia-smi -L | grep -c '^GPU'); fi
WANT_ID=$(grep -v '^#' "$BUNDLE/pins/METHOD_ID" | head -1)
say () { echo "[$(date +%H:%M:%S)] $*" | tee -a "$OUT/run.log"; }
die () { echo "FATAL: $*" | tee -a "$OUT/run.log" >&2; exit 1; }
[ "$(sha256sum "$BUNDLE/method.bin" | cut -c1-64)" = "$(grep -v '^#' "$BUNDLE/pins/GUEST_ELF_SHA256" | head -1)" ] || die "method.bin is not the pinned ELF"
say "CARDS: $G x $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)${DRY:+ (DRY: dev mode)}; N=$N"
for v in $(compgen -e | grep '^HAZYNC_'); do unset "$v"; done
[ "${DRY:-0}" = 1 ] || unset RISC0_DEV_MODE
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  say "=== build ==="
  ( cd "$P" && HAZYNC_GUEST_ELF="$BUNDLE/method.bin" HAZYNC_GUEST_PIN_DIR="$BUNDLE/pins" HAZYNC_LIBZYNC=1 \
      cargo build --release --features cuda -p host ) >"$OUT/build.log" 2>&1 || die "build failed, see $OUT/build.log"
fi
"$BIN" method-id | grep -qw "$WANT_ID" || die "METHOD_ID is not the pinned $WANT_ID"
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-1} HAZYNC_HINT_THREADS=${HAZYNC_HINT_THREADS:-$(nproc)}
( nvidia-smi --query-gpu=index,utilization.gpu,memory.used --format=csv,noheader -l 1 >"$OUT/smi.log" 2>/dev/null ) & SMI=$!
trap 'kill $SMI 2>/dev/null' EXIT

# run a file of commands (one per line) in batches of G, job k of a batch on card k: never two on one card.
# Waits on the batch's OWN pids: a bare `wait` also waits for the nvidia-smi logger, which never exits (it hung the
# first GPU run after leaf 1; the laptop dry run has no nvidia-smi, so its logger died at once and hid this).
pool () {
  local k=0 line pids=()
  while IFS= read -r line; do
    ( CUDA_VISIBLE_DEVICES=$k bash -c "$line" ) & pids+=($!); k=$((k + 1))
    if [ "$k" -ge "$G" ]; then wait "${pids[@]}"; pids=(); k=0; fi
  done < "$1"
  [ ${#pids[@]} -eq 0 ] || wait "${pids[@]}"
}

say "=== leaves: blocks 1..$N, each a base step on the state before it ==="; T0=$SECONDS
: > "$OUT/leaves.cmd"
for h in $(seq 1 "$N"); do
  a=genesis; [ "$h" -gt 1 ] && a="$BUNDLE/anchors/state_$((h - 1)).json"
  echo "cd $P && $BIN libzync-prove-range $h $h $BUNDLE/blocks $a $OUT/leaf/$h > $OUT/leaf/$h.log 2>&1; echo \"\$? $h\" >> $OUT/leaf_exit.txt" >> "$OUT/leaves.cmd"
done
pool "$OUT/leaves.cmd"
LEAF_S=$((SECONDS - T0))
bad=$(awk '$1 != 0' "$OUT/leaf_exit.txt" | wc -l)
[ "$bad" = 0 ] && [ "$(wc -l < "$OUT/leaf_exit.txt")" = "$N" ] || die "$bad leaves failed, see $OUT/leaf/*.log"
say "leaves: $N proved in $LEAF_S s wall on $G GPUs"

say "=== join tree ==="; cur=(); for h in $(seq 1 "$N"); do cur+=("$OUT/leaf/$h/block_$h.rcpt"); done
lvl=0; joins=0; : > "$OUT/joins.tsv"
while [ ${#cur[@]} -gt 1 ]; do
  lvl=$((lvl + 1)); nxt=(); : > "$OUT/l$lvl.cmd"; T0=$SECONDS
  for ((i = 0; i < ${#cur[@]}; i += 2)); do
    if [ $((i + 1)) -lt ${#cur[@]} ]; then
      o="$OUT/join/l${lvl}_$i.rcpt"
      echo "cd $P && $BIN libzync-join ${cur[$i]} ${cur[$((i + 1))]} $o > $o.log 2>&1 || echo FAIL >> $o.log" >> "$OUT/l$lvl.cmd"
      nxt+=("$o"); joins=$((joins + 1))
    else nxt+=("${cur[$i]}"); fi
  done
  pool "$OUT/l$lvl.cmd"
  for o in "${nxt[@]}"; do case "$o" in *join*) [ -s "$o" ] && ! grep -q '^FAIL' "$o.log" || die "join $o failed, see $o.log"
    sed -n 's/^JOINED .*: \([0-9]*\) cycles, \([0-9.]*\) s$/\1\t\2/p' "$o.log" | sed "s|^|$lvl\t$(basename "$o")\t|" >> "$OUT/joins.tsv";; esac; done
  say "level $lvl: ${#nxt[@]} proofs in $((SECONDS - T0)) s"
  cur=("${nxt[@]}")
done
[ "$joins" = $((N - 1)) ] || die "joins $joins != N-1"

say "=== root ==="
R=$("$BIN" journal-sha "${cur[0]}" 2>&1 | awk '/JOURNAL_SHA256/{print $2}')   # journal-sha verifies the receipt first
[ -n "$R" ] || die "the root receipt does not verify against this image"
L=$(cat "$BUNDLE/LINEAR_SHA")
say "root   $R"; say "linear $L"
[ "$R" = "$L" ] || die "the root journal is not the linear chain's block_$N"
say "IDENTICAL: yes (a real join tree of $N leaves, $joins joins, $lvl levels)"
say "leaf time (prove-range, per leaf): $(grep -ho 'proved in [0-9.]* s' "$OUT"/leaf/*.log | awk '{s+=$3; n++; if ($3 > m) m = $3} END{printf "%d leaves, mean %.1f s, max %.1f s", n, (n ? s/n : 0), m}')"
awk -F'\t' '{c+=$3; t+=$4; n++; if ($1 > 1) {c2+=$3; n2++}} END{printf "join cycles: %d joins, mean %d cycles, mean %.1f s; joins of joins: %d, mean %d cycles\n", n, (n ? c/n : 0), (n ? t/n : 0), n2, (n2 ? c2/n2 : 0)}' "$OUT/joins.tsv" | tee -a "$OUT/run.log"
ls -la "${cur[0]}" | awk '{print "root receipt bytes: "$5}' | tee -a "$OUT/run.log"
say "GPU_JOIN_PASS. Harvest $OUT BEFORE releasing the pod."
