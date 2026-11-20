#!/bin/bash
# The tip mode's PRODUCTION witness source (docs/libzync/UTREEXO_ZAPP.md §10.5.1: `zyncapp utreexo-tip-prove --source
# forest`), executed (no proving, no dev mode) on mainnet block files 1..100,000: the same chain of steps as
# tip-exec-tests.sh run twice, once from the forest source and once from the test-scale native replay, whose witnesses
# must be the SAME BYTES (frames_sha256) and whose statements the same roots, each step a new process restarting from
# the previous step's saved forest; then restarts (a kept copy, a corrupt file, a file of the other source), a node
# that lies about a prevout (refused by the source), and two lying sources the guest must refuse.
#   tip-source-tests.sh <zyncapp> <block dir 1..100,000> <state_10000.json> <state_100000.json> <work dir>
set -uo pipefail
Z=$1; B=$2; S10=$3; S100=$4; W=$5
mkdir -p "$W"; unset RISC0_DEV_MODE
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
field () { grep -h -o "$1 .*" "$2" | head -1 | grep -o " $3=[^ ]*" | cut -d= -f2; }

"$Z" utreexo-prove "$B" 1999 "$S10" "$W/base" --execute > "$W/base.log" 2>&1; t "base walk to 1,999" 0 $?
J=$W/base/mainnet-1999.journal
prev=$J; p=1999
rm -f "$W/forest.bin" "$W/native.bin"
for spec in 10000:$S10 20000:$S100 50000:$S100 91722:$S100 91812:$S100 91842:$S100 91880:$S100 100000:$S100; do
  h=${spec%%:*}; s=${spec#*:}
  [ "$h" = 100000 ] && cp "$W/forest.bin" "$W/forest_91880.bin"
  [ "$h" = 91722 ] && cp "$W/forest.bin" "$W/forest_50000.bin"
  "$Z" utreexo-tip-prove "$B" "$prev" "$h" "$s" "$W/f_$h" --execute --source forest --cache "$W/forest.bin" > "$W/f_$h.log" 2>&1
  t "forest source: step $p -> $h" 0 $?
  "$Z" utreexo-tip-prove "$B" "$prev" "$h" "$s" "$W/n_$h" --execute --source native --cache "$W/native.bin" > "$W/n_$h.log" 2>&1
  t "native source: step $p -> $h" 0 $?
  fs=$(field UTREEXO_TIP_WITNESS "$W/f_$h.log" frames_sha256); ns=$(field UTREEXO_TIP_WITNESS "$W/n_$h.log" frames_sha256)
  t "  the same witness bytes from both sources ($p -> $h)" "$ns" "$fs"
  t "  the same roots" "$(field UTREEXO_TIP_JOURNAL "$W/n_$h.log" roots)" "$(field UTREEXO_TIP_JOURNAL "$W/f_$h.log" roots)"
  [ "$p" = 1999 ] || t "  the forest source restarted from the previous step's file (no replay)" 1 "$(grep -c 'UTREEXO_TIP_CACHE hit' "$W/f_$h.log")"
  grep -h -E 'UTREEXO_TIP_(WITNESS|CACHE|REPLAY|EXEC)' "$W/f_$h.log"
  prev=$W/f_$h.journal; p=$h
done
[ -n "$fs" ] || { echo "FAIL no witness digest"; fails=$((fails + 1)); }

# restarts of the last step 91,880 -> 100,000: from a kept copy (a hit, the same bytes); from a corrupt copy (refused,
# a replay from genesis, the same bytes); from the native source's file (the other kind: a replay); from an OLDER kept
# forest (50,000: replayed forward 50,001..91,880, review F1 b); from a forest ABOVE the previous height (100,000: a
# replay from genesis)
want=$(field UTREEXO_TIP_WITNESS "$W/f_100000.log" frames_sha256)
cp "$W/forest.bin" "$W/forest_100000.bin"
for c in copy corrupt other below above; do
  cp "$W/forest_91880.bin" "$W/r_$c.bin"
  [ $c = below ] && cp "$W/forest_50000.bin" "$W/r_$c.bin"
  [ $c = above ] && cp "$W/forest_100000.bin" "$W/r_$c.bin"
  [ $c = corrupt ] && printf '\x01' | dd of="$W/r_$c.bin" bs=1 seek=$(( $(stat -c %s "$W/r_$c.bin") / 2 )) conv=notrunc status=none
  [ $c = other ] && cp "$W/native.bin" "$W/r_$c.bin"
  "$Z" utreexo-tip-prove "$B" "$W/f_91880.journal" 100000 "$S100" "$W/r_$c" --execute --source forest --cache "$W/r_$c.bin" --cache-out "$W/r_$c.out" > "$W/r_$c.log" 2>&1
  t "restart ($c): the step 91,880 -> 100,000 again" 0 $?
  t "  the same witness bytes" "$want" "$(field UTREEXO_TIP_WITNESS "$W/r_$c.log" frames_sha256)"
  case $c in
    copy) t "  a cache hit" 1 "$(grep -c 'UTREEXO_TIP_CACHE hit at 91880' "$W/r_$c.log")";;
    corrupt) t "  the corrupt file refused, replayed from genesis" "1 1" "$(grep -c 'UTREEXO_TIP_CACHE unusable (forest source' "$W/r_$c.log") $(grep -c 'UTREEXO_TIP_REPLAY 1..91880' "$W/r_$c.log")";;
    other) t "  the other source's file refused, replayed" "1 1" "$(grep -c 'UTREEXO_TIP_CACHE unusable.*other witness source' "$W/r_$c.log") $(grep -c 'UTREEXO_TIP_REPLAY 1..91880' "$W/r_$c.log")";;
    below) t "  started from the forest at 50,000 and replayed 50,001..91,880 only" "1 1" "$(grep -c 'UTREEXO_TIP_CACHE below at 50000' "$W/r_$c.log") $(grep -c 'UTREEXO_TIP_REPLAY 50001..91880' "$W/r_$c.log")";;
    above) t "  a forest above the previous height is not used: replayed from genesis" "1 1" "$(grep -c 'not at or below the previous step' "$W/r_$c.log") $(grep -c 'UTREEXO_TIP_REPLAY 1..91880' "$W/r_$c.log")";;
  esac
  grep -h -E 'UTREEXO_TIP_(CACHE|REPLAY)' "$W/r_$c.log"
done

# a node that LIES about a prevout: the block files 1..9,000 as links, one block's first prevout value changed. The
# source refuses it before anything is executed (exit 1, the leaf is not in the forest); control: the honest files
L=$W/lying-blocks; rm -rf "$L"; mkdir -p "$L"
for h in $(seq 1 9000); do ln -s "$B/block_$h.json" "$L/block_$h.json"; done
k=$(python3 - "$B" <<'PY'
import json, sys
for h in range(2000, 9001):
    v = json.load(open(f'{sys.argv[1]}/block_{h}.json'))
    if any(p['coin_height'] < h for t in v['txs'] for p in t['prevouts']):
        print(h); break
PY
)
rm "$L/block_$k.json"
python3 - "$B/block_$k.json" "$L/block_$k.json" "$k" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); h = int(sys.argv[3])
p = next(p for t in v['txs'] for p in t['prevouts'] if p['coin_height'] < h)
p['value'] += 1
json.dump(v, open(sys.argv[2], 'w'))
PY
"$Z" utreexo-tip-prove "$L" "$J" 9000 "$S10" "$W/lie" --execute --source forest > "$W/lie_node.log" 2>&1
t "a lying node (block $k's first prevout value + 1): refused by the source" "1 1" "$? $(grep -c 'is not a live leaf of the forest' "$W/lie_node.log")"
grep -h -m1 'not a live leaf' "$W/lie_node.log" | cut -c1-240
"$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/ctl" --execute --source forest > "$W/ctl.log" 2>&1; t "control: the honest step 1,999 -> 9,000" 0 $?

# lying SOURCES that skip their own checks (test only): the GUEST must refuse (exit 3)
for lie in node store; do
  UTREEXO_TIP_SOURCE_LIE=$lie "$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/neg" --execute --source forest > "$W/neg_$lie.log" 2>&1; e=$?
  t "a lying source ($lie): refused by the guest" "3 1" "$e $(grep -c 'UTREEXO_TIP_GUEST_REFUSED.*do not open' "$W/neg_$lie.log")"
  grep -h -m1 UTREEXO_TIP_GUEST_REFUSED "$W/neg_$lie.log" | cut -c1-220
done
UTREEXO_TIP_SOURCE_LIE=node "$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/neg" --source forest > "$W/neg_prove.log" 2>&1
t "a lying source is never allowed to PROVE" "1 1" "$? $(grep -c 'execute-only' "$W/neg_prove.log")"
[ "$fails" -eq 0 ] && echo UTREEXO_TIP_SOURCE_TESTS_OK || { echo "UTREEXO_TIP_SOURCE_TESTS_FAILED ($fails)"; exit 1; }
