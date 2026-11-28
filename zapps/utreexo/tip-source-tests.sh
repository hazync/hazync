#!/bin/bash
# The tip mode's PRODUCTION witness source (docs/libzync/UTREEXO_ZAPP.md §10.5.1: `zyncapp utreexo-tip-prove --source
# forest`), EXECUTED on the OpenVM engine (no proving): the same chain of steps as tip-exec-tests.sh run twice, once from
# the forest source and once from the test-scale native replay, whose witnesses must be the SAME BYTES (frames_sha256)
# and whose statements the same roots, each step a new process restarting from the previous step's saved forest; then
# restarts (a kept copy, a corrupt file, a file of the other source), a node that lies about a prevout (refused by the
# source), and two lying sources the guest must refuse. Each input is written with --stream and run by `hzengine
# execute` on METERING images of utreexo and utreexo-tip (bench-noverify: env::verify takes the previous statement and
# the core journal without a proof, as RISC Zero's execute did; CUTOVER C8 removed that path).
#   tip-source-tests.sh <zyncapp> <hzengine> <utreexo metering keys> <utreexo-tip metering keys> <block dir>
#                       <state A> <state B> <work dir>
# Heights (env): BASE_H (1999), STEPS ("10000:A 20000:B 50000:B 91722:B 91812:B 91842:B 91880:B 100000:B": the last is
# the restart step, the one before it its previous step, OLD (50000) an older kept forest), LIE_H (9000: the lying-node
# and lying-source steps from the base, under A; the node's lie is in a block above BASE_H). The engine chain 1..200:
# BASE_H=50 STEPS="100:A 120:A 150:A 170:A 200:A" OLD=120 LIE_H=90.
set -uo pipefail
Z=$1; E=$2; KU=$3; KT=$4; B=$5; SA=$6; SB=$7; W=$8
BASE_H=${BASE_H:-1999}; STEPS=${STEPS:-10000:A 20000:B 50000:B 91722:B 91812:B 91842:B 91880:B 100000:B}
OLD=${OLD:-50000}; LIE_H=${LIE_H:-9000}
LAST=$(echo $STEPS | tr ' ' '\n' | tail -1 | cut -d: -f1); PREV=$(echo $STEPS | tr ' ' '\n' | tail -2 | head -1 | cut -d: -f1)
LASTS=$(echo $STEPS | tr ' ' '\n' | tail -1 | cut -d: -f2)
NEXT_OLD=$(echo $STEPS | tr ' ' '\n' | cut -d: -f1 | awk -v o=$OLD 'f{print;exit} $1==o{f=1}')
mkdir -p "$W"; S=$(cd "$(dirname "$0")/../../scripts/libzync" && pwd)
st () { [ "$1" = A ] && echo "$SA" || echo "$SB"; }
UID_=$(python3 -c "import json,hashlib;i=json.load(open('$KU/image.json'));print(hashlib.sha256(bytes.fromhex(i['exe']+i['vm'])).hexdigest())")
xec () { "$E" execute --dir "$1" --input "$2" --self-id-at 0 --journal "$3" --log "$4" > "$4.exec" 2>&1; echo $?; }
got () { python3 "$S/utreexo_stmt.py" "$1" "$2" | grep -o "roots=[^ ]*" | cut -d= -f2; }
why () { sed 's/\x1b\[[0-9;]*m//g' "$1" 2>/dev/null | grep -a -A1 "panicked at" | grep -av "panicked at\|^--$" | head -1 | cut -c1-200; }
# tip <prev journal> <H> <state> <name> [zyncapp flags...]: stream then execute; prints "<host rc> <engine rc>"
tip () { local pj=$1 h=$2 s=$3 n=$4; shift 4
  "$Z" utreexo-tip-prove "$B" "$pj" "$h" "$s" "$W/$n" --stream "$W/$n.bin" --base-id "$UID_" "$@" > "$W/$n.log" 2>&1; local hr=$?
  local er=-; [ $hr = 0 ] && er=$(xec "$KT" "$W/$n.bin" "$W/$n.journal" "$W/$n.glog"); echo "$hr $er"; }
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
field () { grep -h -o "$1 .*" "$2" | head -1 | grep -o " $3=[^ ]*" | cut -d= -f2; }

"$Z" utreexo-prove "$B" "$BASE_H" "$SA" "$W/base" --stream "$W/base.bin" > "$W/base.log" 2>&1; t "base stream to $BASE_H" 0 $?
t "base walk to $BASE_H executed" 0 "$(xec "$KU" "$W/base.bin" "$W/base.journal" "$W/base.glog")"
J=$W/base.journal
prev=$J; p=$BASE_H
rm -f "$W/forest.bin" "$W/native.bin"
for spec in $STEPS; do
  h=${spec%%:*}; s=$(st "${spec#*:}")
  [ "$h" = "$LAST" ] && cp "$W/forest.bin" "$W/forest_$PREV.bin"
  [ "$h" = "$NEXT_OLD" ] && cp "$W/forest.bin" "$W/forest_$OLD.bin"
  t "forest source: step $p -> $h (stream, execute)" "0 0" "$(tip "$prev" "$h" "$s" "f_$h" --source forest --cache "$W/forest.bin")"
  t "native source: step $p -> $h (stream, execute)" "0 0" "$(tip "$prev" "$h" "$s" "n_$h" --source native --cache "$W/native.bin")"
  fs=$(field UTREEXO_TIP_WITNESS "$W/f_$h.log" frames_sha256); ns=$(field UTREEXO_TIP_WITNESS "$W/n_$h.log" frames_sha256)
  t "  the same witness bytes from both sources ($p -> $h)" "$ns" "$fs"
  t "  the same roots in the guests' statements" "$(got tip "$W/n_$h.journal")" "$(got tip "$W/f_$h.journal")"
  [ "$p" = "$BASE_H" ] || t "  the forest source restarted from the previous step's file (no replay)" 1 "$(grep -c 'UTREEXO_TIP_CACHE hit' "$W/f_$h.log")"
  grep -h -E 'UTREEXO_TIP_(WITNESS|CACHE|REPLAY)' "$W/f_$h.log" | cut -c1-200
  prev=$W/f_$h.journal; p=$h
done
[ -n "$fs" ] || { echo "FAIL no witness digest"; fails=$((fails + 1)); }

# restarts of the last step PREV -> LAST: from a kept copy (a hit, the same bytes); from a corrupt copy (refused, a
# replay from genesis, the same bytes); from the native source's file (the other kind: a replay); from an OLDER kept
# forest (OLD: replayed forward OLD+1..PREV, review F1 b); from a forest ABOVE the previous height (LAST: a replay from
# genesis)
want=$(field UTREEXO_TIP_WITNESS "$W/f_$LAST.log" frames_sha256)
cp "$W/forest.bin" "$W/forest_$LAST.bin"
for c in copy corrupt other below above; do
  cp "$W/forest_$PREV.bin" "$W/r_$c.bin"
  [ $c = below ] && cp "$W/forest_$OLD.bin" "$W/r_$c.bin"
  [ $c = above ] && cp "$W/forest_$LAST.bin" "$W/r_$c.bin"
  [ $c = corrupt ] && printf '\x01' | dd of="$W/r_$c.bin" bs=1 seek=$(( $(stat -c %s "$W/r_$c.bin") / 2 )) conv=notrunc status=none
  [ $c = other ] && cp "$W/native.bin" "$W/r_$c.bin"
  t "restart ($c): the step $PREV -> $LAST again (stream, execute)" "0 0" "$(tip "$W/f_$PREV.journal" "$LAST" "$(st "$LASTS")" "r_$c" --source forest --cache "$W/r_$c.bin" --cache-out "$W/r_$c.out")"
  t "  the same witness bytes" "$want" "$(field UTREEXO_TIP_WITNESS "$W/r_$c.log" frames_sha256)"
  case $c in
    copy) t "  a cache hit" 1 "$(grep -c "UTREEXO_TIP_CACHE hit at $PREV" "$W/r_$c.log")";;
    corrupt) t "  the corrupt file refused, replayed from genesis" "1 1" "$(grep -c 'UTREEXO_TIP_CACHE unusable (forest source' "$W/r_$c.log") $(grep -c "UTREEXO_TIP_REPLAY 1..$PREV" "$W/r_$c.log")";;
    other) t "  the other source's file refused, replayed" "1 1" "$(grep -c 'UTREEXO_TIP_CACHE unusable.*other witness source' "$W/r_$c.log") $(grep -c "UTREEXO_TIP_REPLAY 1..$PREV" "$W/r_$c.log")";;
    below) t "  started from the forest at $OLD and replayed $((OLD + 1))..$PREV only" "1 1" "$(grep -c "UTREEXO_TIP_CACHE below at $OLD" "$W/r_$c.log") $(grep -c "UTREEXO_TIP_REPLAY $((OLD + 1))..$PREV" "$W/r_$c.log")";;
    above) t "  a forest above the previous height is not used: replayed from genesis" "1 1" "$(grep -c 'not at or below the previous step' "$W/r_$c.log") $(grep -c "UTREEXO_TIP_REPLAY 1..$PREV" "$W/r_$c.log")";;
  esac
  grep -h -E 'UTREEXO_TIP_(CACHE|REPLAY)' "$W/r_$c.log"
done

# --cache-required (the follower's flag, review round 2 #2): the corrupt copy and the forest above P are EXIT 4, before
# anything is replayed or written (the same files without the flag replay from genesis, above)
for c in corrupt above; do
  "$Z" utreexo-tip-prove "$B" "$W/f_$PREV.journal" "$LAST" "$(st "$LASTS")" "$W/q_$c" --stream "$W/q_$c.bin" --base-id "$UID_" --source forest --cache "$W/r_$c.bin" \
    --cache-required --cache-out "$W/q_$c.out" > "$W/q_$c.log" 2>&1
  t "--cache-required with the $c forest: exit 4, refused, nothing replayed" "4 1 0" "$? $(grep -c UTREEXO_TIP_CACHE_REFUSED "$W/q_$c.log") $(grep -c UTREEXO_TIP_REPLAY "$W/q_$c.log")"
done
"$Z" utreexo-tip-prove "$B" "$W/f_$PREV.journal" "$LAST" "$(st "$LASTS")" "$W/q_ok" --stream "$W/q_ok.bin" --base-id "$UID_" --source forest --cache "$W/r_copy.bin" \
  --cache-required --cache-out "$W/q_ok.out" > "$W/q_ok.log" 2>&1
t "  control: --cache-required with the good forest of $PREV runs" "0 1" "$? $(grep -c "UTREEXO_TIP_CACHE hit at $PREV" "$W/q_ok.log")"

# a node that LIES about a prevout: the block files 1..LIE_H as links, one block's first prevout value changed. The
# source refuses it before any stream is written (exit 1, the leaf is not in the forest); control: the honest files
L=$W/lying-blocks; rm -rf "$L"; mkdir -p "$L"
for h in $(seq 1 "$LIE_H"); do ln -s "$B/block_$h.json" "$L/block_$h.json"; done
k=$(python3 - "$B" "$BASE_H" "$LIE_H" <<'PY'
import json, sys
for h in range(int(sys.argv[2]) + 1, int(sys.argv[3]) + 1):
    v = json.load(open(f'{sys.argv[1]}/block_{h}.json'))
    if any(p['coin_height'] < h for t in v['txs'] for p in t['prevouts']):
        print(h); break
PY
)
if [ -z "$k" ]; then echo "not covered: no block in $((BASE_H + 1))..$LIE_H spends an older coin (the lying node needs one)"; else
rm "$L/block_$k.json"
python3 - "$B/block_$k.json" "$L/block_$k.json" "$k" <<'PY'
import json, sys
v = json.load(open(sys.argv[1])); h = int(sys.argv[3])
p = next(p for t in v['txs'] for p in t['prevouts'] if p['coin_height'] < h)
p['value'] += 1
json.dump(v, open(sys.argv[2], 'w'))
PY
"$Z" utreexo-tip-prove "$L" "$J" "$LIE_H" "$SA" "$W/lie" --stream "$W/lie.bin" --base-id "$UID_" --source forest > "$W/lie_node.log" 2>&1
t "a lying node (block $k's first prevout value + 1): refused by the source" "1 1" "$? $(grep -c 'is not a live leaf of the forest' "$W/lie_node.log")"
grep -h -m1 'not a live leaf' "$W/lie_node.log" | cut -c1-240
fi
t "control: the honest step $BASE_H -> $LIE_H (stream, execute)" "0 0" "$(tip "$J" "$LIE_H" "$SA" ctl --source forest)"

# lying SOURCES that skip their own checks (test only): the GUEST must refuse (the engine's execute fails, the guest's
# panic names the deletions that do not open)
for lie in node store; do
  r=$(UTREEXO_TIP_SOURCE_LIE=$lie tip "$J" "$LIE_H" "$SA" "neg_$lie" --source forest)
  t "a lying source ($lie): refused by the guest" "0 refused yes" "${r%% *} $([ "${r#* }" != 0 ] && echo refused || echo accepted) $(why "$W/neg_$lie.glog" | grep -q 'do not open' && echo yes)"
  why "$W/neg_$lie.glog"
done
# (The RISC Zero host refused to PROVE a lying source's input; the engine host only writes streams, and a proof of such
# a stream cannot exist: the guest refuses it, above.)
[ "$fails" -eq 0 ] && echo UTREEXO_TIP_SOURCE_TESTS_OK || { echo "UTREEXO_TIP_SOURCE_TESTS_FAILED ($fails)"; exit 1; }
