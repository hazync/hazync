#!/bin/bash
# libzync CI: real blocks and Core's own vectors on the published mainnet image (execute mode).
#   1. edge blocks, each valid: the first retarget, both BIP30 repeats, activation boundaries, the largest legacy
#      transaction (364292), segwit activation (481824);
#   2. Core 28's script_tests / tx_valid / tx_invalid with every flag variant (cases made by core_vectors.py from the
#      Core tree the guest is built from): the libsecp path and the batched path with honest hints agree with Core on
#      every case; under forged hints (four variants) nothing Core rejects passes;
#   3. negative tests on block 962,000 (negatives.py), each beside a passing control.
# usage: blocks.sh <mainnet host> <core_vectors.json> <core_vectors_base.json> <out dir>
. "$(dirname "$0")/lib.sh"
H=$(readlink -f "${1:?mainnet host}"); CV=$(readlink -f "${2:?vectors}"); CVB=$(readlink -f "${3:?base vectors}")
O=${4:?out dir}; mkdir -p "$O"; O=$(readlink -f "$O")
unset RISC0_DEV_MODE
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=${HAZYNC_CHUNKS:-4} HAZYNC_HINT_THREADS=${HAZYNC_HINT_THREADS:-2}
$H method-id | head -1

step "1. edge blocks"
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  f=$T/edge/block_$h.json; [ -f "$f" ] || f=$Z/prover/testdata/boundaries/block_$h.json
  HAZYNC_BLOCK=$f $H libzync-hints "$O/edge_$h.lzb" > "$O/edge_$h.hints.log" 2>&1
  HAZYNC_BLOCK=$f HAZYNC_LZ_HINTS=$O/edge_$h.lzb $H agg-exec-fake > "$O/edge_$h.log" 2>&1; rc=$?
  if [ $rc = 0 ] && grep -aq 'block validation cycles' "$O/edge_$h.log" && ! grep -aq 'INVALID\|panicked' "$O/edge_$h.log"; then
    pass "edge $h valid ($(grep -ao 'block validation cycles *[0-9]*' "$O/edge_$h.log" | awk '{print $NF}') aggregate cycles)"
  else fail "edge $h (exit $rc)"; grep -a 'INVALID\|panicked' "$O/edge_$h.log" | head -2 | sed 's/^/    /'; fi
done

step "2. Core's script and transaction vectors"
for m in libsecp honest; do
  a=(); [ $m = honest ] && a=(--sig honest)
  $H core-vectors "$CV" 500 "${a[@]}" > "$O/vectors_$m.log" 2>&1; echo "exit $?"
  L=$(grep -a '^CORE_VECTORS' "$O/vectors_$m.log" | tail -1); echo "$L"
  echo "$L" | grep -q ', 0 differ, of ' && pass "vectors [$m]: every case agrees with Core" || fail "vectors [$m]: '$L'"
done
for v in forged forged-odd forged-rn forged-mixed; do
  $H core-vectors "$CVB" 500 --sig $v > "$O/vectors_$v.log" 2>&1; echo "exit $?"
  L=$(grep -a '^CORE_VECTORS' "$O/vectors_$v.log" | tail -1); echo "$L"
  echo "$L" | grep -q ', 0 differ, of ' && pass "vectors [$v]: nothing Core rejects passes" || fail "vectors [$v]: '$L'"
done

step "3. negative tests on block 962,000"
python3 "$Z/scripts/libzync/ci/negatives.py" "$H" "$Z/prover/block_962000.json" "$O/neg" > "$O/neg.log" 2>&1
cat "$O/neg.log"
want "$O/neg.log" "NEGATIVES_OK" "every negative refused, every control passed"
finish blocks
