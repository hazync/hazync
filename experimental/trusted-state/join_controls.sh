#!/bin/bash
# join_controls.sh <ts-anchor> <ts-ingest-host> <ts-ingest-host, controls build> <anchored tip state.json> <work dir> [<anchor A.json>]
# -- EXPERIMENTAL. The join receipt guest (docs/libzync/TRUSTED_STATE.md §13.3) on regtest H = 104, dev mode:
#   main@104  = the REAL receipt scripts/zapp-regtest/receipts/a/block_104.rcpt (image eb2cd424, verified first);
#   ingest    = the chunked ingest of Core's utxo_104.dat (3 chunks + combine), inline BIP30 tree and split (d 4, 3 pieces);
#   tip       = the anchored lineage's state at 112 (core regtest image, dev-mode run on the anchor; enters as a claim).
# Every control is driven both ways: refused by the real join image, accepted by the controls build with its check cut.
# <anchor A.json>: ts-anchor build's anchor at 104 (default: anchor_104.json next to the tip's out_anc directory), for the
# native `ts-anchor join` lines.
# TIP_IMAGE: the image the anchored run used (default: a4344798..., the PoC's laptop regtest build).
# Prints JOIN_CONTROLS_OK or JOIN_CONTROLS_FAILED <n>.
set -uo pipefail
TS=$1; IH=$2; IHC=$3; TIP=$(readlink -f "$4"); W=$5
ANCHOR=$(readlink -f "${6:-$(dirname "$TIP")/../anchor_104.json}")
Z=$(cd "$(dirname "$0")/../.." && pwd); G=$Z/scripts/zapp-regtest
TIP_IMAGE=${TIP_IMAGE:-a4344798574f7811046bfe5a6916ad1eaa1fc3b9262cd6959480b87d8e110ecc}
MAIN_IMAGE=$(awk '{print $2}' "$G/receipts/method_id.txt")
mkdir -p "$W"; cd "$W" || exit 1
export RISC0_DEV_MODE=1 TS_TIP_IMAGE=$TIP_IMAGE
GEN=1; TAIL=2; MATCH=4; LINK=8; SHAPE=16; SELF=32; ALL=63
tsa() { env -u RISC0_DEV_MODE "$TS" "$@"; }
python3 "$Z/experimental/trusted-state/side_inputs.py" "$G/blocks/main" 104 side > /dev/null
ingest() {  # ingest <host> <dir>: chunks + combine inline + combine split
  local h=$1 d=$2; rm -rf "$d"; tsa chunk-plan --network regtest --snapshot "$G/oracle/utxo_104.dat" --side side --chunks 3 --out "$d" > /dev/null
  for f in "$d"/chunk_*.in; do "$h" chunk "$f" "${f%.in}.rcpt" > /dev/null 2>&1 || return 1; done
  tsa bip30-plan --final "$d/final.in" --depth 4 --pieces 3 --out "$d" > /dev/null
  for f in "$d"/piece_*.in; do "$h" bip30 "$f" "${f%.in}.rcpt" > /dev/null 2>&1 || return 1; done
  "$h" combine "$d/final.in" "$d/expected.journal" "$d"/chunk_{0,1,2}.rcpt > /dev/null 2>&1 || return 1
  "$h" combine "$d/final.in" "$d/expected.journal" "$d"/chunk_{0,1,2}.rcpt --bip30 "$d"/piece_{0,1,2}.rcpt > /dev/null 2>&1 || return 1
}
ingest "$IH" I || { echo "ingest (real images) failed"; exit 1; }
ingest "$IHC" IC || { echo "ingest (controls build) failed"; exit 1; }
inputs() { tsa join-inputs --main "$G/receipts/a/block_$1.rcpt" --main-image "$MAIN_IMAGE" --anchored-tip "$2" --out "$3" "${@:4}" | tail -1; }
inputs 104 "$TIP" J --tip-image "$TIP_IMAGE"
inputs 104 "$TIP" Jtip --forge-tip-anchor
inputs 105 "$TIP" J105
inputs 104 "$TIP" Jgen --forge-main-anchor
inputs 104 "$G/receipts/a/state_104.json" Jlow        # "tip" = main@104 itself: not above H, genesis-anchored
inputs 104 "$TIP" Jsm --forge-main-self-id            # main@H's journal with self_id != main_image, all else as committed
inputs 104 "$TIP" Jst --forge-tip-self-id             # the anchored tip's journal with self_id != tip_image
bad=0
join() {  # join <host> <cut> <dir> <ingest receipt> [env...]: prints JOINED or refused <why>
  local h=$1 cut=$2 d=$3 r=$4; shift 4
  local o; o=$(env TS_CUT=$cut "$@" "$h" join "$d" "$r" 2>&1)
  if echo "$o" | grep -aq "^JOINED"; then echo "JOINED $(echo "$o" | grep -a "^JOINED" | cut -c1-150)"
  else echo "refused $(echo "$o" | grep -a -o "panicked.*\|Guest panicked: .*" | tail -1 | sed 's/.*Guest panicked: //' | cut -c1-110)"; fi
}
want() { case "$3" in "$2"*) echo "PASS $1: $3";; *) echo "FAIL $1: got '$3', wanted '$2'"; bad=$((bad+1));; esac; }
want "honest, inline BIP30 ingest"         JOINED "$(join "$IH" 0 J I/final.in.rcpt)"
want "honest, split BIP30 ingest"          JOINED "$(join "$IH" 0 J I/final.in.split.rcpt)"
want "controls build, no cut"              JOINED "$(join "$IHC" 0 J IC/final.in.split.rcpt)"
ctl() {  # ctl <label> <cut> <dir> <ingest> [env...]
  local label=$1 cut=$2 d=$3 r=$4; shift 4
  want "$label: real join image"    refused "$(join "$IH" 0 "$d" "I/$r" "$@")"
  want "$label: controls, no cut"   refused "$(join "$IHC" 0 "$d" "IC/$r" "$@")"
  want "$label: controls, cut $cut" JOINED  "$(join "$IHC" "$cut" "$d" "IC/$r" "$@")"
}
ctl "tip on another anchor (anchor_id != link of the ingested anchor)" $LINK Jtip final.in.split.rcpt
ctl "main@105 joined with the ingest at 104"                            $((MATCH|LINK)) J105 final.in.split.rcpt
ctl "main@H not genesis-anchored (dev-mode claim)"                       $GEN Jgen final.in.split.rcpt
ctl "the verifier pins another chunk image"                              $TAIL J final.in.split.rcpt "TS_CHUNK_IMAGE=$(printf '%064d' 7)"
ctl "the verifier expects an inline tree, the ingest is split"           $TAIL J final.in.split.rcpt TS_BIP30_IMAGE=0
# main@104's own state as the "tip": its self_id is main_image, so it is told tip_image = main_image (the self_id check
# then holds and the two checks cut here are the only ones it fails); each left on alone still refuses it
ctl "\"tip\" at H itself, genesis-anchored"                              $((SHAPE|LINK)) Jlow final.in.split.rcpt "TS_TIP_IMAGE=$MAIN_IMAGE"
want "\"tip\" at H itself: controls, cut $SHAPE only (link on)" refused "$(join "$IHC" $SHAPE Jlow IC/final.in.split.rcpt "TS_TIP_IMAGE=$MAIN_IMAGE")"
want "\"tip\" at H itself: controls, cut $LINK only (shape on)"  refused "$(join "$IHC" $LINK Jlow IC/final.in.split.rcpt "TS_TIP_IMAGE=$MAIN_IMAGE")"
# S1: the REAL core image run with self_id = a rogue image commits that self_id; its receipt verifies under the real id
ctl "main@H recursed against another image (self_id != main_image)"     $SELF Jsm final.in.split.rcpt
ctl "anchored tip recursed against another image (self_id != tip_image)" $SELF Jst final.in.split.rcpt
# not a cut check: the anchored tip's claim is under another image than the one the guest is told (env::verify)
# (refused by env::verify itself: no receipt resolves the claim the guest asks for; any other failure is a FAIL)
unresolved() {  # unresolved <label> <host> <cut>
  local o; o=$(env TS_CUT="$3" TS_TIP_CLAIM_IMAGE="$MAIN_IMAGE" "$2" join J "$4" 2>&1)
  if grep -aq "no receipt found to resolve assumption" <<< "$o"; then echo "PASS $1: refused (no receipt found to resolve assumption)"
  else echo "FAIL $1: wanted 'no receipt found to resolve assumption', got '$(grep -a "panicked\|JOINED" <<< "$o" | tail -1 | cut -c1-110)'"; bad=$((bad+1)); fi
}
unresolved "tip claimed under another image: real join image"     "$IH" 0 I/final.in.split.rcpt
unresolved "tip claimed under another image: controls, all cuts"  "$IHC" $ALL IC/final.in.split.rcpt
# the controls build's result lines say so (an id it prints must never be pinned); the real build's do not
lab() {  # lab <controls|real> <line regex> <cmd...>: the result lines must exist; all labelled (controls) or none (real)
  local kind=$1 re=$2; shift 2
  local l; l=$(env TS_CUT=0 "$@" 2>/dev/null | grep -a "$re")
  local n; n=$(grep -ac . <<< "$l"); local t; t=$(grep -ac "CONTROLS BUILD" <<< "$l")
  if [ -z "$l" ]; then echo "FAIL $kind: no '$re' line from $*"; bad=$((bad+1))
  elif { [ "$kind" = controls ] && [ "$t" = "$n" ]; } || { [ "$kind" = real ] && [ "$t" = 0 ]; }; then echo "PASS $kind build's '$re' line(s): $t of $n labelled CONTROLS"
  else echo "FAIL $kind build's '$re' line(s): $t of $n labelled CONTROLS"; bad=$((bad+1)); fi
}
rm -rf L; mkdir -p L/c L/r; cp IC/final.in L/c/; cp I/final.in L/r/   # the combine writes next to its final.in
for k in controls:IHC:IC:c real:IH:I:r; do
  IFS=: read -r kind hv src dd <<< "$k"; hb=${!hv}
  lab "$kind" "^JOINED"         "$hb" join J "$src/final.in.split.rcpt"
  lab "$kind" "^CHUNK "         "$hb" chunk "$src/chunk_0.in" "L/$dd/chunk_0.rcpt"
  lab "$kind" "^BIP30 "         "$hb" bip30 "$src/piece_0.in" "L/$dd/piece_0.rcpt"
  lab "$kind" "^COMBINE image\|^ *BIP30 from" "$hb" combine "L/$dd/final.in" - "$src"/chunk_{0,1,2}.rcpt --bip30 "$src"/piece_{0,1,2}.rcpt
done
# the dev-mode substitution of main@H (main.journal.fake) is said on stdout, and on the JOINED line
o=$("$IHC" join Jgen IC/final.in.split.rcpt 2>/dev/null)   # refused (cut 0); the substitution is said first
if grep -aq "^CONTROL: main@H is a DEV-MODE CLAIM" <<< "$o"; then echo "PASS main.journal.fake substitution on stdout"
else echo "FAIL main.journal.fake substitution not on stdout"; bad=$((bad+1)); fi
[ "$(env TS_CUT=$GEN "$IHC" join Jgen IC/final.in.split.rcpt 2>/dev/null | grep -a "^JOINED" | grep -ac "DEV-MODE CLAIM")" = 1 ] \
  && echo "PASS a join over main.journal.fake says so on its JOINED line" || { echo "FAIL JOINED line over main.journal.fake not labelled"; bad=$((bad+1)); }
# the native paths (ts-anchor): the same self_id check
native() {  # native <label> <want: ok|refused> <cmd...>
  local label=$1 w=$2; shift 2; local got=refused
  tsa "$@" > native.log 2>&1 && got=ok
  if [ "$got" = "$w" ]; then echo "PASS native $label: $got"; else echo "FAIL native $label: $got, wanted $w ($(tail -1 native.log))"; bad=$((bad+1)); fi
}
native "join-inputs, the tip's self_id = --tip-image"  ok      join-inputs --main "$G/receipts/a/block_104.rcpt" --main-image "$MAIN_IMAGE" --anchored-tip "$TIP" --tip-image "$TIP_IMAGE" --out Jn
native "join-inputs, --tip-image another image"        refused join-inputs --main "$G/receipts/a/block_104.rcpt" --main-image "$MAIN_IMAGE" --anchored-tip "$TIP" --tip-image "$MAIN_IMAGE" --out Jn
native "join-receipt, honest"                          ok      join-receipt --main "$G/receipts/a/block_104.rcpt" --main-image "$MAIN_IMAGE" --ingest-journal I/final.in.journal --anchored-tip "$TIP" --tip-image "$TIP_IMAGE"
native "join-receipt, --tip-image another image"       refused join-receipt --main "$G/receipts/a/block_104.rcpt" --main-image "$MAIN_IMAGE" --ingest-journal I/final.in.journal --anchored-tip "$TIP" --tip-image "$MAIN_IMAGE"
native "join, main@H state with self_id = --image"     ok      join --anchor "$ANCHOR" --main "$G/receipts/a/state_104.json" --image "$MAIN_IMAGE" --tip "$TIP" --tip-image "$TIP_IMAGE"
native "join, main@H state with self_id != --image"    refused join --anchor "$ANCHOR" --main "$G/receipts/a/state_104.json" --image "$TIP_IMAGE" --tip "$TIP" --tip-image "$TIP_IMAGE"
native "join, the tip's self_id != --tip-image"        refused join --anchor "$ANCHOR" --main "$G/receipts/a/state_104.json" --image "$MAIN_IMAGE" --tip "$TIP" --tip-image "$MAIN_IMAGE"
[ $bad = 0 ] && echo JOIN_CONTROLS_OK || { echo "JOIN_CONTROLS_FAILED $bad"; exit 1; }
