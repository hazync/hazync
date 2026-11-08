#!/bin/bash
# libzync CI: the guest against Bitcoin Core itself, then handovers.
#   1. the rig (experimental/libzync-bench/core_rules/rig.py): a real Bitcoin Core 28.0 bitcoind on regtest and the
#      regtest guest are given the same crafted blocks (built with Core's own test_framework), one per rule; Core's
#      submitblock answer is the reference and every case must agree, accept or reject;
#   2. handover O -> N on the rig's chain (handover/run.sh): N continues O's chain at 101 by a handover, refuses
#      anything else, and every guest check is reached alone and rejects with its own message.
# usage: rig.sh <regtest host O> <regtest host N> <bitcoind 28.0> <Core 28.0 source tree> <out dir>
. "$(dirname "$0")/lib.sh"
HO=$(readlink -f "${1:?host O}"); HN=$(readlink -f "${2:?host N}"); BD=$(readlink -f "${3:?bitcoind}"); CS=$(readlink -f "${4:?Core source}")
O=${5:?out dir}; mkdir -p "$O"; O=$(readlink -f "$O")
"$BD" -version | head -1; $HO method-id | head -1; $HN method-id | head -1

step "1. rig vs Bitcoin Core 28.0 (regtest)"
rm -rf "$O/rig"; mkdir -p "$O/rig"
RIG_COINLOG=1 BITCOIND=$BD HZ_CORE_SRC=$CS PYTHONUNBUFFERED=1 RIG_VERBOSE=1 python3 "$B/core_rules/rig.py" "$HO" "$O/rig" > "$O/rig.log" 2>&1
echo "exit $?"
L=$(grep -a '^RIG [0-9]* cases' "$O/rig.log" | tail -1); echo "$L"
n=$(echo "$L" | awk '{print $2}'); a=$(echo "$L" | awk '{print $4}')
if [ -n "$n" ] && [ "$n" -ge 96 ] && [ "$a" = "$n" ] && echo "$L" | grep -q ', 0 deferred, 0 to look at$'; then pass "rig: $a / $n agree with Core"
else fail "rig: '$L'"; grep -a '^[A-Z-]\{4,\} ' "$O/rig.log" | grep -v '^AGREE' | head -10 | sed 's/^/    /'; fi

step "2. handover O -> N"
bash "$B/handover/run.sh" "$HO" "$HN" "$O/rig/blocks/main" "$O/ho" > "$O/ho.log" 2>&1; rc=$?
L=$(grep -a '^handover run.sh: ' "$O/ho.log" | tail -1); echo "$L (exit $rc)"
k=$(echo "$L" | sed -n 's#.*: \([0-9]*\)/\([0-9]*\) as expected#\1 \2#p')
if [ $rc = 0 ] && [ -n "$k" ] && [ "${k% *}" = "${k#* }" ] && [ "${k% *}" -ge 16 ]; then pass "handover: $L"
else fail "handover: '$L'"; grep -a '^WRONG' "$O/ho.log" | head -10 | sed 's/^/    /'; fi
finish rig
