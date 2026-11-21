#!/bin/bash
# join_controls.sh <ts-anchor> <ts-ingest-host> <ts-ingest-host, controls build> <anchored tip state.json> <work dir>
# -- EXPERIMENTAL. The join receipt guest (docs/libzync/TRUSTED_STATE.md §13.3) on regtest H = 104, dev mode:
#   main@104  = the REAL receipt scripts/zapp-regtest/receipts/a/block_104.rcpt (image eb2cd424, verified first);
#   ingest    = the chunked ingest of Core's utxo_104.dat (3 chunks + combine), inline BIP30 tree and split (d 4, 3 pieces);
#   tip       = the anchored lineage's state at 112 (core regtest image, dev-mode run on the anchor; enters as a claim).
# Every control is driven both ways: refused by the real join image, accepted by the controls build with its check cut.
# TIP_IMAGE: the image the anchored run used (default: a4344798..., the PoC's laptop regtest build).
# Prints JOIN_CONTROLS_OK or JOIN_CONTROLS_FAILED <n>.
set -uo pipefail
TS=$1; IH=$2; IHC=$3; TIP=$(readlink -f "$4"); W=$5
Z=$(cd "$(dirname "$0")/../.." && pwd); G=$Z/scripts/zapp-regtest
TIP_IMAGE=${TIP_IMAGE:-a4344798574f7811046bfe5a6916ad1eaa1fc3b9262cd6959480b87d8e110ecc}
MAIN_IMAGE=$(awk '{print $2}' "$G/receipts/method_id.txt")
mkdir -p "$W"; cd "$W" || exit 1
export RISC0_DEV_MODE=1 TS_TIP_IMAGE=$TIP_IMAGE
GEN=1; TAIL=2; MATCH=4; LINK=8; SHAPE=16
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
inputs 104 "$TIP" J
inputs 104 "$TIP" Jtip --forge-tip-anchor
inputs 105 "$TIP" J105
inputs 104 "$TIP" Jgen --forge-main-anchor
inputs 104 "$G/receipts/a/state_104.json" Jlow        # "tip" = main@104 itself: not above H, genesis-anchored
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
ctl "\"tip\" at H itself, genesis-anchored"                              $((SHAPE|LINK)) Jlow final.in.split.rcpt
# not a cut check: the anchored tip's claim is under another image than the one the guest is told (env::verify)
want "tip claimed under another image: real join image" refused "$(join "$IH" 0 J I/final.in.split.rcpt TS_TIP_CLAIM_IMAGE=$MAIN_IMAGE)"
want "tip claimed under another image: controls, all cuts" refused "$(join "$IHC" 31 J IC/final.in.split.rcpt TS_TIP_CLAIM_IMAGE=$MAIN_IMAGE)"
[ $bad = 0 ] && echo JOIN_CONTROLS_OK || { echo "JOIN_CONTROLS_FAILED $bad"; exit 1; }
