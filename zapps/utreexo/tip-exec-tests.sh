#!/bin/bash
# Tip mode (docs/libzync/UTREEXO_ZAPP.md §10), executed (no proving, no dev mode) on mainnet block files: a base walk to
# 1,999, then a chain of steps to 100,000, each compared with Floresta's own code (utreexo-ref) where it printed the
# height, and with the base walk at the same height; then the guest's refusals. CPU, minutes; needs the block files
# 1..100,000 and prep's core states (the published host's) at 10,000 and 100,000.
#   tip-exec-tests.sh <zyncapp> <block dir> <state_10000.json> <state_100000.json> <utreexo-ref log> <work dir>
set -uo pipefail
Z=$1; B=$2; S10=$3; S100=$4; REF=$5; W=$6
mkdir -p "$W"; unset RISC0_DEV_MODE
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
roots () { grep -h -o "$1.*" "$2" | grep -o 'roots=[^ ]*' | head -1; }

# the base: the walk of §3 to 1,999 under the core state at 10,000 (an unresolved assumption)
"$Z" utreexo-prove "$B" 1999 "$S10" "$W/base" --execute --ref "$REF" > "$W/base.log" 2>&1; t "base walk to 1,999" 0 $?
J=$W/base/mainnet-1999.journal
prev=$J; p=1999
for spec in 10000:$S10 20000:$S100 50000:$S100 91722:$S100 91812:$S100 91842:$S100 91880:$S100 100000:$S100; do
  h=${spec%%:*}; s=${spec#*:}
  "$Z" utreexo-tip-prove "$B" "$prev" "$h" "$s" "$W/step_$h" --execute --prof 1000 --cache "$W/forest.bin" --ref "$REF" > "$W/step_$h.log" 2>&1
  t "step $p -> $h (== Floresta's update_acc at $h)" 0 $?
  grep -h -E 'UTREEXO_TIP_EXEC|UTREEXO_REF_MATCH|UTX_TIP_CYC h='"$h"' ' "$W/step_$h.log"
  prev=$W/step_$h.journal; p=$h
done
# the base walk straight to 10,000 gives the same roots as base(1,999) + step(10,000): two designs, one accumulator
"$Z" utreexo-prove "$B" 10000 "$S10" "$W/base10" --execute > "$W/base10.log" 2>&1; t "base walk to 10,000" 0 $?
t "walk(10,000) == walk(1,999) + step(10,000)" "$(roots UTREEXO_JOURNAL "$W/base10.log")" "$(roots UTREEXO_TIP_JOURNAL "$W/step_10000.log")"

# refusals: every case must be refused by the GUEST (exit 3, UTREEXO_TIP_GUEST_REFUSED) for the reason named
"$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/ctl" --execute > "$W/ctl.log" 2>&1; t "control: the same step untampered" 0 $?
OPEN="the deletions do not open in the accumulator"; NOREC="no receipt found to resolve assumption"
refused () { # refused <case> <reason substring> <previous journal> <H> <core state>
  UTREEXO_TIP_TAMPER=$1 "$Z" utreexo-tip-prove "$B" "$3" "$4" "$5" "$W/neg" --execute > "$W/neg_$1_$4.log" 2>&1; local e=$?
  local line; line=$(grep -h -m1 UTREEXO_TIP_GUEST_REFUSED "$W/neg_$1_$4.log")
  t "refused: $1 (H = $4): $2" "3 yes" "$e $(echo "$line" | grep -qF -- "$2" && echo yes)"
  echo "$line" | cut -c1-220
}
# (H = 9,000 from the base at 1,999, under the core state at 10,000: spends and 1,000 link headers)
for spec in "spent_value:$OPEN" "spent_block:$OPEN" "spent_code:$OPEN" "spent_coinbase:$OPEN" "spent_drop:truncated" \
            "spent_extra:more records than deletions" "proof_hash:$OPEN" "proof_target:$OPEN" "proof_empty:$OPEN" \
            "tx_swap:Merkle root" "first_prev:header does not follow the previous block" \
            "link:does not reach the core tip" "link_last:the link from block H does not end at the core tip" \
            "link_drop:link headers above H: want T - H" "prev_roots:$NOREC" "prev_kind:$NOREC" "core_id:$NOREC"; do
  refused "${spec%%:*}" "${spec#*:}" "$J" 9000 "$S10"
done
# from a STEP (prev kind 2): the step 10,000 -> 15,000 under the core state at 100,000
for spec in "spent_value:$OPEN" "first_prev:header does not follow the previous block" "prev_roots:$NOREC" "prev_kind:$NOREC"; do
  refused "${spec%%:*}" "${spec#*:}" "$W/step_10000.journal" 15000 "$S100"
done
# a tamper case with nothing to tamper is a host error, never a pass: link_drop with H = T
UTREEXO_TIP_TAMPER=link_drop "$Z" utreexo-tip-prove "$B" "$J" 10000 "$S10" "$W/neg" --execute > "$W/neg_nolink.log" 2>&1
t "link_drop with H = T is a host error (nothing to drop), not a refusal" "1 1" "$? $(grep -c 'nothing to drop' "$W/neg_nolink.log")"
[ "$fails" -eq 0 ] && echo UTREEXO_TIP_EXEC_TESTS_OK || { echo "UTREEXO_TIP_EXEC_TESTS_FAILED ($fails)"; exit 1; }
