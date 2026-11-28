#!/bin/bash
# The hints zyncAPP's tests (docs/libzync/HINTS_ZAPP.md §8), EXECUTED on the OpenVM engine (no proof): `zyncapp
# hints-prove --stream` writes the guest's input, `hzengine execute` runs the hints image with the core chain's REAL
# engine proof as its child (in-guest verification included); the statement is read from the execution's journal.
# (The RISC Zero version executed with the core receipt as an unresolved assumption; CUTOVER C8 removed that path.)
#   1. the BIP's Elias-Fano test vectors (hints_ref.py);
#   2. format 1: the native file == hints_ref.py's (an independent generator from hintsfile.md), byte for byte, and the
#      guest's committed hints_sha256 == that file's;
#   3. format 2: the native file == the hintsfile crate 0.2.0's own writer on hints_ref.py's indices, byte for byte,
#      the crate reads it back to the same indices, and the guest commits its hash;
#   4. counted / unspent / value in the statement == hints_ref.py's, and the value == the core state's utxo_value;
#   5. every negative case refused IN THE GUEST (the engine's execute fails with the guest's panic in its log).
# usage: hints_test.sh <zyncapp> <hzengine> <hints keys dir> <core proof P (P.proof.json + P.journal, or HZE1)>
#                      <the core image's keys dir (HZ_CHILD_KEYS)> <hints-crate-ref> <block dir> <H> <out dir>
set -u
ZA=$(readlink -f "$1"); E=$(readlink -f "$2"); K=$(readlink -f "$3"); CP=$(readlink -f "$4"); CK=$(readlink -f "$5")
CR=$(readlink -f "$6"); B=$(readlink -f "$7"); H=$8; O=$9
mkdir -p "$O"; O=$(readlink -f "$O"); Z=$(cd "$(dirname "$0")/../.." && pwd)
CJ=$CP.journal; [ -f "$CJ" ] || { python3 -c "import sys,struct;b=open(sys.argv[1],'rb').read();assert b[:4]==b'HZE1';n=struct.unpack('<I',b[4:8])[0];open(sys.argv[2],'wb').write(b[8+n:])" "$CP" "$O/core.journal" && CJ=$O/core.journal; }
P=0; F=0
pass() { echo "PASS $*"; P=$((P+1)); }
fail() { echo "FAIL $*"; F=$((F+1)); }
grab() { sed -n "s/.*$2=\([^ ]*\).*/\1/p" "$1" | head -1; }
# exec <name> [zyncapp hints-prove args...]: stream, then the engine; rc of the engine; the guest's log in $O/<name>.glog
xrun() { local n=$1; shift
  "$ZA" hints-prove "$B" "$H" "$CJ" "$@" --stream "$O/$n.bin" > "$O/$n.host.log" 2>&1
  [ -s "$O/$n.bin" ] || { echo "nostream"; return; }
  HZ_CHILD_KEYS=$CK "$E" execute --dir "$K" --input "$O/$n.bin" --child "$CP" --journal "$O/$n.j" --log "$O/$n.glog" > "$O/$n.exec.log" 2>&1; echo $?; }
# the statement's fields from a journal (risc0 serde words: u32 each, a byte per word in [u8; N], u64 as two words)
stmt() { python3 - "$1" <<'PY'
import sys, struct
w = list(struct.unpack(f"<{len(open(sys.argv[1],'rb').read())//4}I", open(sys.argv[1],'rb').read())); i = 0
def u32():
    global i; i += 1; return w[i-1]
def u64():
    lo, hi = u32(), u32(); return lo | hi << 32
def b32():
    global i; i += 32; return bytes(w[i-32:i]).hex()
kind, fmt, height, block, sha, n, counted, unspent, value = u32(), u32(), u32(), b32(), b32(), u64(), u64(), u64(), u64()
print(f"hints_sha256={sha} counted={counted} unspent={unspent} value={value} height={height} format={fmt}")
PY
}

python3 "$Z/scripts/libzync/hints_ref.py" vectors "$Z/prover/testdata/libzync/hints/elias_fano.json" > "$O/vectors.log" 2>&1
grep -q '^HINTS_REF_VECTORS OK' "$O/vectors.log" && pass "BIP Elias-Fano test vectors (3)" || fail "BIP vectors: $(tail -1 "$O/vectors.log")"

python3 "$Z/scripts/libzync/hints_ref.py" make "$B" "$H" "$O/ref-bip.hints" --order bip --indices "$O/ref.json" > "$O/ref.log" 2>&1
python3 "$Z/scripts/libzync/hints_ref.py" from-indices "$O/ref.json" "$H" "$O/ref-crate.hints" --order crate >> "$O/ref.log" 2>&1
"$CR" write "$O/ref.json" "$H" "$O/crate.hints" > "$O/crate.log" 2>&1
cmp -s "$O/ref-crate.hints" "$O/crate.hints" && pass "hints_ref.py (crate order) == the crate's writer" || fail "hints_ref.py != the crate"

for f in 1 2; do
  rc=$(xrun f$f "$O/f$f" --format $f)
  [ "$rc" = 0 ] || { fail "format $f: the guest refused the honest input (rc $rc: $(grep -a -m1 -A1 'panicked at' "$O/f$f.glog" | tail -1))"; continue; }
  stmt "$O/f$f.j" > "$O/f$f.stmt"
  js=$(grab "$O/f$f.stmt" hints_sha256); fs=$(sha256sum "$O/f$f/mainnet-$H.hints" | cut -d' ' -f1)
  [ "$js" = "$fs" ] && pass "format $f: the guest's committed hash == the written file's" || fail "format $f: journal $js != file $fs"
done
cmp -s "$O/f1/mainnet-$H.hints" "$O/ref-bip.hints" && pass "format 1: the file == hints_ref.py's, byte for byte ($(stat -c%s "$O/ref-bip.hints") bytes)" || fail "format 1 != hints_ref.py"
cmp -s "$O/f2/mainnet-$H.hints" "$O/crate.hints" && pass "format 2: the file == the hintsfile crate's, byte for byte" || fail "format 2 != the crate"
"$CR" read "$O/f2/mainnet-$H.hints" "$O/back.json" > /dev/null 2>&1
python3 -c "import json,sys; sys.exit(json.load(open('$O/back.json')) != json.load(open('$O/ref.json')))" && pass "the crate reads format 2 back to hints_ref.py's indices" || fail "crate read-back differs"
c=$(grab "$O/f1.stmt" counted); u=$(grab "$O/f1.stmt" unspent); v=$(grab "$O/f1.stmt" value)
rc=$(grab "$O/ref.log" counted); ru=$(grab "$O/ref.log" unspent); rv=$(grab "$O/ref.log" value)
[ "$c $u $v" = "$rc $ru $rv" ] && pass "the statement's counted / unspent / value == hints_ref.py ($c / $u / $v sat)" || fail "counts $c $u $v != ref $rc $ru $rv"
# the core state's utxo_value is the value live at ITS height T: comparable only when H = T
ci=$("$ZA" core-info "$CJ"); sv=$(echo "$ci" | grep -ao 'utxo_value=[0-9]*' | cut -d= -f2); ct=$(echo "$ci" | sed -n 's/^CORE height=\([0-9]*\) .*/\1/p')
if [ "$ct" = "$H" ]; then [ "$v" = "$sv" ] && pass "unspent value == the core state's utxo_value" || fail "value $v != the core state's $sv"
else echo "(H = $H below the core tip $ct: the core state's utxo_value is at $ct, not compared)"; fi

for c in spent_as_unspent unspent_as_spent wrong_spender tracked state undo header dup_tx dup_last drop_tx swap_tx witness_tx lineage pre_undo core_id format assumevalid; do
  rc=$(HINTS_TAMPER=$c xrun neg_$c "$O/neg")
  if ! grep -q "^HINTS_TAMPER $c\|HINTS_NATIVE_REFUSED\|^HINTS_STREAM" "$O/neg_$c.host.log"; then echo "SKIP $c: $(grep -a -m1 -E 'zyncapp:|HINTS_TAMPER' "$O/neg_$c.host.log")"; continue; fi
  why=$(sed 's/\x1b\[[0-9;]*m//g' "$O/neg_$c.glog" 2>/dev/null | grep -a -A1 "panicked at" | grep -av "panicked at\|^--$" | head -1 | cut -c1-110)
  if [ "$rc" = nostream ]; then fail "no stream for $c ($(tail -1 "$O/neg_$c.host.log"))"
  elif [ "$rc" != 0 ] && [ -n "$why" ]; then pass "refused in the guest: $c ($why)"
  elif [ "$rc" != 0 ]; then fail "refused, NOT by the guest: $c (rc $rc: $(tail -2 "$O/neg_$c.exec.log" | tr '\n' ' ' | cut -c1-150))"
  else fail "NOT refused: $c"; fi
done
echo "HINTS_TEST $P passed, $F failed"
[ $F = 0 ]
