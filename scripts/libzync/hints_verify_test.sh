#!/bin/bash
# The hints reader's refusals (docs/libzync/HINTS_ZAPP.md §8) on a REAL engine hints proof, through the CLI a node runs
# (zapp-hints, zapps/hints). The RISC Zero version drove `zyncapp hints-verify` and a RISC Zero receipt; CUTOVER C8
# removed both. Defaults: the repo's fixture (zapps/hints/tests/data/engine: H = 200 over keys-ch's core proof of
# mainnet 1..200, DEMO: below minimum work), another H's file made here by hints_ref.py from the repo's mainnet blocks,
# and the core chain's own proof as "a proof of another image".
# usage: hints_verify_test.sh <zapp-hints> [fixture dir] [out dir]
set -u
SV=$(readlink -f "$1"); Z=$(cd "$(dirname "$0")/../.." && pwd)
V=$(readlink -f "${2:-$Z/zapps/hints/tests/data/engine}"); O=${3:-$(mktemp -d)}; mkdir -p "$O"
H=200; X=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
CORE=c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9   # keys-ch, the fixture's core image
R=$V/proof.hze; F=$V/mainnet-$H.hints; U=$V/mainnet-$H.undo; OTHERP=$Z/swiftsync-verify/tests/data/engine200/proof.hze
K=(--image "$(cat "$V/image_id")" --vk "$V/vk.bin" --vk-sha256 "$(sha256sum "$V/vk.bin" | cut -c1-64)" --core-image "$CORE")
P=0; N=0
run() { "$SV" "${K[@]}" "$@" > "$O/last.log" 2>&1; echo $?; }
ok() {   # ok <name> <args...>: exit 0 (OK) or 2 (DEMO: a valid statement about a chain below minimum work)
  local n=$1; shift; e=$(run "$@")
  if [ "$e" = 0 ] || { [ "$e" = 2 ] && grep -q '^SWIFTSYNC_VERIFY_DEMO' "$O/last.log"; }; then echo "PASS $n (exit $e)"; P=$((P+1));
  else echo "FAIL $n (exit $e) $(tail -1 "$O/last.log")"; N=$((N+1)); fi
}
no() {   # no <name> <args...>: must be refused (exit 1, SWIFTSYNC_VERIFY_FAIL), never an abort
  local n=$1; shift; e=$(run "$@")
  if [ "$e" = 1 ] && grep -q '^SWIFTSYNC_VERIFY_FAIL' "$O/last.log"; then echo "PASS refused: $n ($(grep -a -m1 -i 'fail\|refus\|not ' "$O/last.log" | cut -c1-120))"; P=$((P+1));
  else echo "FAIL NOT refused: $n (exit $e) $(tail -1 "$O/last.log")"; N=$((N+1)); fi
}
# another H's file (150), format 1, from the repo's mainnet blocks: an honest file for the wrong height
python3 "$Z/scripts/libzync/hints_ref.py" make "$Z/prover/testdata/libzync/mainnet" 150 "$O/other.hints" --order bip --indices "$O/other.json" > "$O/other.log" 2>&1 \
  || { echo "FAIL hints_ref.py make 150: $(tail -1 "$O/other.log")"; N=$((N+1)); }
ok "the proven file, its undo file, --expect" --expect "$H:$X" --undo "$U" "$R" "$F"
python3 -c "import sys;b=bytearray(open('$F','rb').read());b[-1]^=1;open('$O/flip.hints','wb').write(b)"
python3 -c "import sys;b=open('$F','rb').read();open('$O/trunc.hints','wb').write(b[:-1])"
# the same indices, one CompactSize written non-minimally (decodes to the same lists in a lenient reader)
python3 -c "
b=open('$F','rb').read(); i=9; n=b[i]
assert n < 0xfd
open('$O/noncanon.hints','wb').write(b[:i]+bytes([0xfd,n,0])+b[i+1:])"
python3 -c "import sys;b=bytearray(open('$U','rb').read());b[0]^=1;open('$O/flip.undo','wb').write(b)"
python3 -c "open('$O/tiny.hints','wb').write(bytes([0x55,0x54,0x58,0x4f,0])+(2**32-1).to_bytes(4,'little'))"
no "a 9-byte file claiming H = 2^32 - 1 (FAIL, not an abort)" --expect "$H:$X" "$R" "$O/tiny.hints"
no "a flipped bit" --expect "$H:$X" "$R" "$O/flip.hints"
no "a truncated file" --expect "$H:$X" "$R" "$O/trunc.hints"
no "a non-canonical re-encoding of block 1's list" --expect "$H:$X" "$R" "$O/noncanon.hints"
no "the file for another H (150)" --expect "$H:$X" "$R" "$O/other.hints"
no "a tampered undo file" --expect "$H:$X" --undo "$O/flip.undo" "$R" "$F"
no "--expect: another hash at H" --expect "$H:$(printf '0%.0s' {1..63})1" "$R" "$F"
no "--expect: another height" --expect "$((H-1)):$X" "$R" "$F"
no "a proof of another image (the core chain's own proof)" --expect "$H:$X" "$OTHERP" "$F"
e=$("$SV" --image "$(cat "$V/image_id")" --vk "$V/vk.bin" --vk-sha256 "$(sha256sum "$V/vk.bin" | cut -c1-64)" \
      --core-image "$(printf 'ab%.0s' {1..32})" --expect "$H:$X" "$R" "$F" > "$O/last.log" 2>&1; echo $?)
[ "$e" = 1 ] && grep -q '^SWIFTSYNC_VERIFY_FAIL' "$O/last.log" && { echo "PASS refused: an untrusted core image"; P=$((P+1)); } || { echo "FAIL an untrusted core image (exit $e)"; N=$((N+1)); }
echo "HINTS_VERIFY_TEST $P passed, $N failed"
[ $N = 0 ]
