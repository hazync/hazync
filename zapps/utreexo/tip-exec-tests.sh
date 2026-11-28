#!/bin/bash
# Tip mode (docs/libzync/UTREEXO_ZAPP.md §10), EXECUTED on the OpenVM engine (no proving): a base walk to BASE_H, then a
# chain of steps, each guest statement compared with the zAPP host's native expectation (and with Floresta's own code,
# utreexo-ref, where its log prints the height), and the walk straight to the first step's height compared with base +
# step; then the guest's refusals. `zyncapp ... --stream` writes each input, `hzengine execute` runs it on METERING images
# of utreexo and utreexo-tip (built with HZ_GUEST_FEATURES=bench-noverify: env::verify accepts the previous run and the
# core journal without a proof, as RISC Zero's execute took them as unresolved assumptions; CUTOVER C8 removed that
# path). So the cases that only a proof can refuse (a previous statement or core journal changed: env::verify, the
# deferral circuit's job) are listed as not covered here, never as passes.
#   tip-exec-tests.sh <zyncapp> <hzengine> <utreexo metering keys> <utreexo-tip metering keys> <block dir>
#                     <core state A> <core state B> <utreexo-ref log | -> <work dir>
# The heights (env): BASE_H (1999), STEPS ("10000:A 20000:B 50000:B 91722:B 91812:B 91842:B 91880:B 100000:B", A/B the
# core state), NEG_H (9000, under A, from the base), NEG2 ("10000:15000": from that step, under B), CTL_H (9000).
# Mainnet 1..100,000 needs those block files and prep's states at 10,000 and 100,000; the engine chain 1..200 runs it with
# BASE_H=50 STEPS="100:A 150:A 200:A" NEG_H=90 NEG2=100:180 CTL_H=90 and A = B = its state at 200.
set -uo pipefail
Z=$1; E=$2; KU=$3; KT=$4; B=$5; SA=$6; SB=$7; REF=$8; W=$9
BASE_H=${BASE_H:-1999}; STEPS=${STEPS:-10000:A 20000:B 50000:B 91722:B 91812:B 91842:B 91880:B 100000:B}
NEG_H=${NEG_H:-9000}; NEG2=${NEG2:-10000:15000}; CTL_H=${CTL_H:-9000}
mkdir -p "$W"; S=$(cd "$(dirname "$0")/../../scripts/libzync" && pwd)
fails=0; notcov=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
st () { [ "$1" = A ] && echo "$SA" || echo "$SB"; }
RA=(); [ "$REF" = - ] || RA=(--ref "$REF")
UID_=$(python3 -c "import json,hashlib;i=json.load(open('$KU/image.json'));print(hashlib.sha256(bytes.fromhex(i['exe']+i['vm'])).hexdigest())")
xec () { # xec <keys> <stream> <journal> <log>: the engine's rc
  "$E" execute --dir "$1" --input "$2" --self-id-at 0 --journal "$3" --log "$4" > "$4.exec" 2>&1; echo $?; }
expect () { grep -h -o "expect leaves=[0-9]* roots=[^ ]*" "$1" | head -1 | sed 's/^expect //'; }
got () { python3 "$S/utreexo_stmt.py" "$1" "$2" | grep -o "leaves=[0-9]* roots=[^ ]*"; }
why () { sed 's/\x1b\[[0-9;]*m//g' "$1" 2>/dev/null | grep -a -A1 "panicked at" | grep -av "panicked at\|^--$" | head -1 | cut -c1-200; }

# the base: the walk of §3 to BASE_H under core state A
"$Z" utreexo-prove "$B" "$BASE_H" "$SA" "$W/base" --stream "$W/base.bin" "${RA[@]}" > "$W/base.log" 2>&1; t "base stream to $BASE_H" 0 $?
t "base walk to $BASE_H executed" 0 "$(xec "$KU" "$W/base.bin" "$W/base.j" "$W/base.glog")"
t "  the guest's statement == the host's" "$(expect "$W/base.log")" "$(got base "$W/base.j")"
J=$W/base.j; prev=$J; p=$BASE_H; first=
for spec in $STEPS; do
  h=${spec%%:*}; s=$(st "${spec#*:}"); first=${first:-$h}
  "$Z" utreexo-tip-prove "$B" "$prev" "$h" "$s" "$W/step_$h" --stream "$W/step_$h.bin" --base-id "$UID_" --prof 1000 --cache "$W/forest.bin" "${RA[@]}" > "$W/step_$h.log" 2>&1
  t "step $p -> $h: stream" 0 $?
  t "step $p -> $h: executed" 0 "$(xec "$KT" "$W/step_$h.bin" "$W/step_$h.j" "$W/step_$h.glog")"
  t "  the guest's statement == the host's" "$(expect "$W/step_$h.log")" "$(got tip "$W/step_$h.j")"
  grep -h -E 'UTREEXO_REF_MATCH|UTREEXO_TIP_NATIVE' "$W/step_$h.log" | cut -c1-200
  prev=$W/step_$h.j; p=$h
done
# the base walk straight to the first step's height gives the same roots as base + step: two designs, one accumulator
"$Z" utreexo-prove "$B" "$first" "$SA" "$W/basef" --stream "$W/basef.bin" > "$W/basef.log" 2>&1
t "base walk to $first executed" 0 "$(xec "$KU" "$W/basef.bin" "$W/basef.j" "$W/basef.glog")"
t "walk($first) == walk($BASE_H) + step($first)" "$(got base "$W/basef.j" | grep -o 'roots=.*')" "$(got tip "$W/step_$first.j" | grep -o 'roots=.*')"

# refusals: every case must be refused IN THE GUEST (its panic in the engine's log) for the reason named
"$Z" utreexo-tip-prove "$B" "$J" "$CTL_H" "$SA" "$W/ctl" --stream "$W/ctl.bin" --base-id "$UID_" > "$W/ctl.log" 2>&1
t "control: the same step untampered" 0 "$(xec "$KT" "$W/ctl.bin" "$W/ctl.j" "$W/ctl.glog")"
OPEN="the deletions do not open in the accumulator"
refused () { # refused <case> <reason substring> <previous journal> <H> <core state>
  UTREEXO_TIP_TAMPER=$1 "$Z" utreexo-tip-prove "$B" "$3" "$4" "$5" "$W/neg" --stream "$W/neg.bin" --base-id "$UID_" > "$W/neg_$1_$4.log" 2>&1
  local e; e=$(xec "$KT" "$W/neg.bin" "$W/neg.j" "$W/neg_$1_$4.glog"); local w; w=$(why "$W/neg_$1_$4.glog")
  t "refused in the guest: $1 (H = $4): $2" "1 yes" "$([ "$e" != 0 ] && echo 1 || echo 0) $(echo "$w" | grep -qF -- "$2" && echo yes)"
  echo "     $w"
}
notcovered () { echo "not covered (metering image: env::verify takes any journal; only a proof can refuse it): $1"; notcov=$((notcov + 1)); }
for spec in "spent_value:$OPEN" "spent_block:$OPEN" "spent_code:$OPEN" "spent_coinbase:$OPEN" "spent_drop:truncated" \
            "spent_extra:more records than deletions" "proof_hash:$OPEN" "proof_target:$OPEN" "proof_empty:$OPEN" \
            "tx_swap:Merkle root" "first_prev:header does not follow the previous block" \
            "link:does not reach the core tip" "link_last:the link from block H does not end at the core tip" \
            "link_drop:link headers above H: want T - H"; do
  refused "${spec%%:*}" "${spec#*:}" "$J" "$NEG_H" "$SA"
done
for c in prev_roots prev_kind core_id; do notcovered "$c (H = $NEG_H)"; done
# from a STEP (prev kind 2)
n2f=${NEG2%%:*}; n2h=${NEG2#*:}
for spec in "spent_value:$OPEN" "first_prev:header does not follow the previous block"; do
  refused "${spec%%:*}" "${spec#*:}" "$W/step_$n2f.j" "$n2h" "$SB"
done
for c in prev_roots prev_kind; do notcovered "$c (from the step at $n2f)"; done
# a tamper case with nothing to tamper is a host error, never a pass: link_drop with H = T
tT=$("$Z" core-info "$SA" | sed -n 's/^CORE height=\([0-9]*\) .*/\1/p')
UTREEXO_TIP_TAMPER=link_drop "$Z" utreexo-tip-prove "$B" "$J" "$tT" "$SA" "$W/neg" --stream "$W/neg.bin" --base-id "$UID_" > "$W/neg_nolink.log" 2>&1
t "link_drop with H = T is a host error (nothing to drop), not a refusal" "1 1" "$? $(grep -c 'nothing to drop' "$W/neg_nolink.log")"
echo "($notcov cases not covered on a metering image)"
[ "$fails" -eq 0 ] && echo UTREEXO_TIP_EXEC_TESTS_OK || { echo "UTREEXO_TIP_EXEC_TESTS_FAILED ($fails)"; exit 1; }
