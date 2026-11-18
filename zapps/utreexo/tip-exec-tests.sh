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

# refusals: every case must be refused by the GUEST (exit 3, UTREEXO_TIP_GUEST_REFUSED)
"$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/ctl" --execute > "$W/ctl.log" 2>&1; t "control: the same step untampered" 0 $?
# (H = 9,000 under the core state at 10,000: the step has spends and 1,000 link headers)
for c in spent_value spent_block spent_code spent_coinbase spent_drop spent_extra proof_hash proof_target proof_empty tx_swap link link_drop prev_roots prev_kind core_id; do
  UTREEXO_TIP_TAMPER=$c "$Z" utreexo-tip-prove "$B" "$J" 9000 "$S10" "$W/neg" --execute > "$W/neg_$c.log" 2>&1; e=$?
  t "refused: $c" "3 1" "$e $(grep -c UTREEXO_TIP_GUEST_REFUSED "$W/neg_$c.log")"
  grep -h -m1 UTREEXO_TIP_GUEST_REFUSED "$W/neg_$c.log" | cut -c1-220
done
[ "$fails" -eq 0 ] && echo UTREEXO_TIP_EXEC_TESTS_OK || { echo "UTREEXO_TIP_EXEC_TESTS_FAILED ($fails)"; exit 1; }
