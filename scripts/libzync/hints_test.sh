#!/bin/bash
# The hints zyncAPP's tests (docs/libzync/HINTS_ZAPP.md §8), EXECUTED (no proof, no dev mode): the guest runs every check
# with the core receipt as an unresolved assumption whose journal is prep's state at H (libzync-prep, the published host).
#   1. the BIP's Elias-Fano test vectors (hints_ref.py);
#   2. format 1: the guest's file == hints_ref.py's (an independent generator from hintsfile.md), byte for byte;
#   3. format 2: the guest's file == the hintsfile crate 0.2.0's own writer on hints_ref.py's indices, byte for byte, and
#      the crate reads the guest's file back to the same indices;
#   4. every negative case refused by the guest (and, where it gets that far, by the native derivation first).
# usage: hints_test.sh <zyncapp> <hints-crate-ref> <block dir> <state_H.json> <H> <out dir>
set -u
ZA=$(readlink -f "$1"); CR=$(readlink -f "$2"); B=$(readlink -f "$3"); ST=$(readlink -f "$4"); H=$5; O=$6
mkdir -p "$O"; O=$(readlink -f "$O"); Z=$(cd "$(dirname "$0")/../.." && pwd)
unset RISC0_DEV_MODE
P=0; F=0
pass() { echo "PASS $*"; P=$((P+1)); }
fail() { echo "FAIL $*"; F=$((F+1)); }
grab() { sed -n "s/.*$2=\([^ ]*\).*/\1/p" "$1" | head -1; }

python3 "$Z/scripts/libzync/hints_ref.py" vectors "$Z/prover/testdata/libzync/hints/elias_fano.json" > "$O/vectors.log" 2>&1
grep -q '^HINTS_REF_VECTORS OK' "$O/vectors.log" && pass "BIP Elias-Fano test vectors (3)" || fail "BIP vectors: $(tail -1 "$O/vectors.log")"

python3 "$Z/scripts/libzync/hints_ref.py" make "$B" "$H" "$O/ref-bip.hints" --order bip --indices "$O/ref.json" > "$O/ref.log" 2>&1
python3 "$Z/scripts/libzync/hints_ref.py" from-indices "$O/ref.json" "$H" "$O/ref-crate.hints" --order crate >> "$O/ref.log" 2>&1
"$CR" write "$O/ref.json" "$H" "$O/crate.hints" > "$O/crate.log" 2>&1
cmp -s "$O/ref-crate.hints" "$O/crate.hints" && pass "hints_ref.py (crate order) == the crate's writer" || fail "hints_ref.py != the crate"

for f in 1 2; do
  "$ZA" hints-prove "$B" "$H" "$ST" "$O/f$f" --format $f --execute --prof 1000 > "$O/exec_f$f.log" 2>&1; echo "exec format $f exit $?"
  grep -q '^HINTS_EXEC ' "$O/exec_f$f.log" || { fail "format $f: no execution ($(grep -a -m1 -iE 'error|panic|die|zyncapp:' "$O/exec_f$f.log"))"; continue; }
  js=$(grab "$O/exec_f$f.log" hints_sha256); fs=$(sha256sum "$O/f$f/mainnet-$H.hints" | cut -d' ' -f1)
  [ "$js" = "$fs" ] && pass "format $f: the journal's hash == the written file's" || fail "format $f: journal $js != file $fs"
done
cmp -s "$O/f1/mainnet-$H.hints" "$O/ref-bip.hints" && pass "format 1: the guest's file == hints_ref.py's, byte for byte ($(stat -c%s "$O/ref-bip.hints") bytes)" || fail "format 1 != hints_ref.py"
cmp -s "$O/f2/mainnet-$H.hints" "$O/crate.hints" && pass "format 2: the guest's file == the hintsfile crate's, byte for byte" || fail "format 2 != the crate"
"$CR" read "$O/f2/mainnet-$H.hints" "$O/back.json" > /dev/null 2>&1
python3 -c "import json,sys; sys.exit(json.load(open('$O/back.json')) != json.load(open('$O/ref.json')))" && pass "the crate reads format 2 back to hints_ref.py's indices" || fail "crate read-back differs"
c=$(grab "$O/exec_f1.log" counted); u=$(grab "$O/exec_f1.log" unspent); v=$(grab "$O/exec_f1.log" value)
rc=$(grab "$O/ref.log" counted); ru=$(grab "$O/ref.log" unspent); rv=$(grab "$O/ref.log" value)
[ "$c $u $v" = "$rc $ru $rv" ] && pass "counted / unspent / value == hints_ref.py ($c / $u / $v sat)" || fail "counts $c $u $v != ref $rc $ru $rv"
sv=$(python3 -c "import json;print(json.load(open('$ST'))['utxo_value'])"); [ "$v" = "$sv" ] && pass "unspent value == the core state's utxo_value" || fail "value $v != state $sv"

for c in spent_as_unspent unspent_as_spent wrong_spender tracked state undo header dup_tx dup_last drop_tx swap_tx witness_tx lineage pre_undo core_id format assumevalid; do
  HINTS_TAMPER=$c "$ZA" hints-prove "$B" "$H" "$ST" "$O/neg" --execute > "$O/neg_$c.log" 2>&1; e=$?
  if [ $e = 3 ] && grep -q "^HINTS_GUEST_REFUSED $c" "$O/neg_$c.log"; then
    pass "refused: $c ($(sed -n "s/^HINTS_GUEST_REFUSED $c: //p" "$O/neg_$c.log" | cut -c1-110))"
  elif grep -q "^HINTS_TAMPER $c" "$O/neg_$c.log"; then fail "NOT refused: $c (exit $e)"; else echo "SKIP $c: $(grep -a -m1 -E 'zyncapp:|HINTS_TAMPER' "$O/neg_$c.log")"; fi
done
echo "HINTS_TEST $P passed, $F failed"
[ $F = 0 ]
