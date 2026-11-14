#!/bin/bash
# The hints verifier's refusals (docs/libzync/HINTS_ZAPP.md §8), on a REAL hints receipt (proved, not dev mode).
# usage: hints_verify_test.sh <zyncapp> <hints receipt> <its .hints> <its .undo> <H> <hash at H> <another H's .hints>
#        <a receipt of another image, e.g. the core receipt> <out dir>
set -u
ZA=$1; R=$2; F=$3; U=$4; H=$5; X=$6; OTHER=$7; CORE=$8; O=$9; mkdir -p "$O"
unset RISC0_DEV_MODE
P=0; N=0
run() { "$ZA" hints-verify "$@" > "$O/last.log" 2>&1; echo $?; }
ok() {   # ok <name> <args...>: exit 0 (OK) or 2 (DEMO: a valid statement about a chain below minimum work)
  local n=$1; shift; e=$(run "$@")
  if [ "$e" = 0 ] || { [ "$e" = 2 ] && grep -q '^ZYNCAPP_DEMO: ZYNCAPP_HINTS_OK' "$O/last.log"; }; then echo "PASS $n (exit $e: $(grep -m1 -oE 'ZYNCAPP_(DEMO|HINTS_OK)[^-]*' "$O/last.log" | cut -c1-150))"; P=$((P+1));
  else echo "FAIL $n (exit $e) $(tail -1 "$O/last.log")"; N=$((N+1)); fi
}
no() {   # no <name> <args...>: must be refused (exit 1, ZYNCAPP_FAIL)
  local n=$1; shift; e=$(run "$@")
  if [ "$e" = 1 ] && grep -q '^ZYNCAPP_FAIL' "$O/last.log"; then echo "PASS refused: $n ($(grep -m1 '^ZYNCAPP_FAIL' "$O/last.log" | cut -c15-140))"; P=$((P+1));
  else echo "FAIL NOT refused: $n (exit $e) $(tail -1 "$O/last.log")"; N=$((N+1)); fi
}
ok "the proven file, its undo file, --expect" "$R" "$F" --undo "$U" --expect "$H:$X"
python3 -c "import sys;b=bytearray(open('$F','rb').read());b[-1]^=1;open('$O/flip.hints','wb').write(b)"
python3 -c "import sys;b=open('$F','rb').read();open('$O/trunc.hints','wb').write(b[:-1])"
# the same indices, one CompactSize written non-minimally (decodes to the same lists in a lenient reader)
python3 -c "
b=open('$F','rb').read(); i=9; n=b[i]
assert n < 0xfd
open('$O/noncanon.hints','wb').write(b[:i]+bytes([0xfd,n,0])+b[i+1:])"
python3 -c "import sys;b=bytearray(open('$U','rb').read());b[0]^=1;open('$O/flip.undo','wb').write(b)"
python3 -c "open('$O/tiny.hints','wb').write(bytes([0x55,0x54,0x58,0x4f,0])+(2**32-1).to_bytes(4,'little'))"
no "a 9-byte file claiming H = 2^32 - 1 (FAIL, not an abort)" "$R" "$O/tiny.hints" --expect "$H:$X"
no "a flipped bit" "$R" "$O/flip.hints" --expect "$H:$X"
no "a truncated file" "$R" "$O/trunc.hints" --expect "$H:$X"
no "a non-canonical re-encoding of block 1's list" "$R" "$O/noncanon.hints" --expect "$H:$X"
no "the file for another H" "$R" "$OTHER" --expect "$H:$X"
no "a tampered undo file" "$R" "$F" --undo "$O/flip.undo" --expect "$H:$X"
no "--expect: another hash at H" "$R" "$F" --expect "$H:$(printf '0%.0s' {1..63})1"
no "--expect: another height" "$R" "$F" --expect "$((H-1)):$X"
no "a receipt of another image" "$CORE" "$F" --expect "$H:$X"
no "an untrusted core image" "$R" "$F" --expect "$H:$X" --core-image "$(printf 'ab%.0s' {1..32})"
e=$(RISC0_DEV_MODE=1 "$ZA" hints-verify "$R" "$F" --expect "$H:$X" > "$O/last.log" 2>&1; echo $?)
[ "$e" = 1 ] && grep -q 'RISC0_DEV_MODE' "$O/last.log" && { echo "PASS refused: RISC0_DEV_MODE set"; P=$((P+1)); } || { echo "FAIL dev mode not refused (exit $e)"; N=$((N+1)); }
# the same library through the CLI a node runs: swiftsync-verify hints (SV=<binary>, optional)
if [ -n "${SV:-}" ]; then
  "$SV" hints --expect "$H:$X" --undo "$U" "$R" "$F" > "$O/sv.log" 2>&1; e=$?
  { [ $e = 0 ] || { [ $e = 2 ] && grep -q '^SWIFTSYNC_VERIFY_DEMO' "$O/sv.log"; }; } && { echo "PASS swiftsync-verify hints: the proven file (exit $e)"; P=$((P+1)); } || { echo "FAIL swiftsync-verify hints (exit $e)"; N=$((N+1)); }
  "$SV" hints --expect "$H:$X" "$R" "$O/tiny.hints" > "$O/sv.log" 2>&1; e=$?
  [ $e = 1 ] && grep -q '^SWIFTSYNC_VERIFY_FAIL' "$O/sv.log" && { echo "PASS swiftsync-verify hints refused: the 9-byte file"; P=$((P+1)); } || { echo "FAIL swiftsync-verify hints: the 9-byte file (exit $e)"; N=$((N+1)); }
  RISC0_DEV_MODE=1 "$SV" hints --expect "$H:$X" "$R" "$F" > "$O/sv.log" 2>&1; e=$?
  [ $e = 1 ] && grep -q 'RISC0_DEV_MODE' "$O/sv.log" && { echo "PASS swiftsync-verify hints refused: RISC0_DEV_MODE set"; P=$((P+1)); } || { echo "FAIL swiftsync-verify hints: dev mode (exit $e)"; N=$((N+1)); }
fi
echo "HINTS_VERIFY_TEST $P passed, $N failed"
[ $N = 0 ]
