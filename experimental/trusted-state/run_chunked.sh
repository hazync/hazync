#!/bin/bash
# run_chunked.sh <ts-anchor> <ts-ingest-host> <network> <snapshot> <side prefix> <chunks> <work dir> [<expected journal>] -- EXPERIMENTAL
# The chunked ingest (docs/libzync/TRUSTED_STATE.md §12) end to end, executor in DEV MODE: ts-anchor chunk-plan, one
# process per chunk (its peak RSS by /usr/bin/time), then the combine, whose journal must equal <expected journal> (the
# one-shot ingest's) or else the planner's own expected.journal. TRIE_DIR=<dir> keeps the planner's trie on disk.
# Writes <work>/summary.tsv (chunk, coins, user cycles, padded cycles, segments, peak RSS kB, wall s) and prints
# CHUNKED_OK or CHUNKED_FAILED. CHUNK_COINS=<C> cuts by coin count instead of <chunks> (then <chunks> is an upper bound).
# BIP30_DEPTH=<d> BIP30_PIECES=<P>: the BIP30 tree as P piece receipts at depth d (§13.1), the combine run with --bip30;
# the piece rows go to <work>/pieces.tsv. TIMEV=<cmd> replaces `/usr/bin/time -v` (a box without GNU time).
set -uo pipefail
read -r -a TV <<< "${TIMEV:-/usr/bin/time -v}"
TS=$1; IH=$2; NET=$3; SNAP=$4; SIDE=$5; N=$6; W=$7; EXP=${8:-}
mkdir -p "$W"
export RISC0_DEV_MODE=1
plan=(--network "$NET" --snapshot "$SNAP" --side "$SIDE" --out "$W")
if [ -n "${CHUNK_COINS:-}" ]; then plan+=(--chunk-coins "$CHUNK_COINS"); else plan+=(--chunks "$N"); fi
[ -n "${TRIE_DIR:-}" ] && { rm -rf "$TRIE_DIR"; plan+=(--trie-dir "$TRIE_DIR"); }
"${TV[@]}" env -u RISC0_DEV_MODE "$TS" chunk-plan "${plan[@]}" > "$W/plan.log" 2>&1 || { echo "CHUNKED_FAILED plan"; tail -3 "$W/plan.log"; exit 1; }
grep -E "^CHUNK_PLAN|Maximum resident|Elapsed" "$W/plan.log"
printf "chunk\tcoins\tuser\tpadded\tsegments\tpeak_rss_kb\twall_s\n" > "$W/summary.tsv"
rcpts=()
for ((i = 0; i < N; i++)); do
  [ -f "$W/chunk_$i.in" ] || break
  "${TV[@]}" "$IH" chunk "$W/chunk_$i.in" "$W/chunk_$i.rcpt" > "$W/chunk_$i.log" 2>&1 || { echo "CHUNKED_FAILED chunk $i"; grep -a -m2 "panicked" "$W/chunk_$i.log"; exit 1; }
  rcpts+=("$W/chunk_$i.rcpt")
  awk -v i="$i" '/^CHUNK /{u=$4; p=$6; s=$8} /TSC /{for(k=1;k<=NF;k++) if($k ~ /^n=/) n=substr($k,3)} /Maximum resident/{r=$6} /Elapsed/{w=$8}
       END{split(w,t,":"); sec=(length(t)==3)?t[1]*3600+t[2]*60+t[3]:t[1]*60+t[2]; print i "\t" n "\t" u "\t" p "\t" s "\t" r "\t" sec}' "$W/chunk_$i.log" >> "$W/summary.tsv"
done
[ -n "$EXP" ] || EXP="$W/expected.journal"
bip=()
if [ -n "${BIP30_DEPTH:-}" ]; then
  env -u RISC0_DEV_MODE "$TS" bip30-plan --final "$W/final.in" --depth "$BIP30_DEPTH" --pieces "${BIP30_PIECES:?}" --out "$W" | tail -1
  printf "piece\tentries\trepeatable\tuser\tpadded\tsegments\tpeak_rss_kb\twall_s\n" > "$W/pieces.tsv"
  bip=(--bip30)
  for ((i = 0; i < BIP30_PIECES; i++)); do
    "${TV[@]}" "$IH" bip30 "$W/piece_$i.in" "$W/piece_$i.rcpt" > "$W/piece_$i.log" 2>&1 || { echo "CHUNKED_FAILED piece $i"; grep -a -m2 "panicked" "$W/piece_$i.log"; exit 1; }
    bip+=("$W/piece_$i.rcpt")
    awk -v i="$i" '/^BIP30 /{u=$6; p=$8; s=$10} /TSB /{for(k=1;k<=NF;k++){if($k ~ /^entries=/) e=substr($k,9); if($k ~ /^repeatable=/) r=substr($k,12)}} /Maximum resident/{m=$6} /Elapsed/{w=$8}
         END{split(w,t,":"); sec=(length(t)==3)?t[1]*3600+t[2]*60+t[3]:t[1]*60+t[2]; print i "\t" e "\t" r "\t" u "\t" p "\t" s "\t" m "\t" sec}' "$W/piece_$i.log" >> "$W/pieces.tsv"
  done
  column -t "$W/pieces.tsv"
fi
"${TV[@]}" "$IH" combine "$W/final.in" "$EXP" "${rcpts[@]}" "${bip[@]}" > "$W/combine.log" 2>&1
grep -aE "^COMBINE|TSX|Maximum resident|Elapsed" "$W/combine.log"
column -t "$W/summary.tsv"
awk 'NR>1{u+=$3; p+=$4; s+=$5; if($6>m)m=$6} END{printf "chunks total: user %d padded %d segments %d max peak RSS %d kB\n", u, p, s, m}' "$W/summary.tsv"
grep -aq COMBINE_MATCH "$W/combine.log" && echo CHUNKED_OK || { echo CHUNKED_FAILED; exit 1; }
