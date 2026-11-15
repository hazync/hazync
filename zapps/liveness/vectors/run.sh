#!/bin/bash
# Liveness proofs through the liveness zAPP's CLI (docs/libzync/LIVENESS_ZAPP.md), against the real proof of mainnet
# 1..200 (8888d59c, the core's swiftsync-verify/vectors), and its producer (zapp-liveness prove).
# usage: run.sh <zapp-liveness>     exit codes: 2 DEMO (honest: below minimum work), 1 FAIL
set -u
V=${1:?zapp-liveness}
D=$(cd "$(dirname "$0")" && pwd); C=$D/../../../swiftsync-verify/vectors; R=$C/mainnet_1_200_8888d59c.rcpt
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
IMG="--image 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4"
T9=0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9
T170=f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
pass=0; fail=0
case_() { # <name> <want rc> <args...>
  local n=$1 w=$2; shift 2
  "$V" $IMG "$@" > "$T/out" 2>&1; local rc=$?
  if [ "$rc" = "$w" ]; then pass=$((pass+1)); echo "ok   $n (rc $rc)$( [ "$w" = 1 ] && grep -a -m1 "FAIL\]" "$T/out" | sed "s/.*\.hex\"*: */: /;s/.*rcpt\"*: */: /" | cut -c1-110)"
  else fail=$((fail+1)); echo "FAIL $n (rc $rc, want $w)"; grep -a 'FAIL\]\|liveness' "$T/out" | head -3 | sed 's/^/     /'; fi
}
# tamper <in> <out> <python expression on bytearray b>: edit the decoded proof
tamper() { python3 -c "import sys;b=bytearray(bytes.fromhex(open(sys.argv[1]).read().strip()));$3;open(sys.argv[2],'w').write(b.hex())" "$D/$1" "$T/$2"; }

case_ "block 9 coin: spent at 170 (at 200)"           2 --liveness "$D/b9_0_at200.hex"   "$T9:0@200=spent:170:1:0" "$R"
case_ "block 9 coin: unspent after 169 (opened)"      2 --liveness "$D/b9_0_at169.hex"   "$T9:0@169=unspent" "$R"
case_ "first payment to Hal: unspent at the tip"      2 --liveness "$D/b170_0_at200.hex" "$T170:0@tip=unspent,value=1000000000" "$R"
case_ "first payment change: spent at 181"            2 --liveness "$D/b170_1_at200.hex" "$T170:1@200=spent:181" "$R"
case_ "first payment change: unspent after 170"       2 --liveness "$D/b170_1_at170.hex" "$T170:1@170=unspent" "$R"
case_ "question without a claim (the CLI states it)"  2 --liveness "$D/b9_0_at200.hex"   "$T9:0@200" "$R"
# refusals
case_ "spent coin, question claims unspent"           1 --liveness "$D/b9_0_at200.hex"   "$T9:0@200=unspent" "$R"
case_ "unspent coin, question claims spent"           1 --liveness "$D/b170_0_at200.hex" "$T170:0@200=spent" "$R"
case_ "wrong amount asked"                            1 --liveness "$D/b170_0_at200.hex" "$T170:0@200,value=1000000001" "$R"
case_ "wrong script asked"                            1 --liveness "$D/b170_0_at200.hex" "$T170:0@200,script=51" "$R"
case_ "another outpoint asked"                        1 --liveness "$D/b170_0_at200.hex" "$T170:1@200" "$R"
case_ "another height asked"                          1 --liveness "$D/b170_0_at200.hex" "$T170:0@199" "$R"
tamper b9_0_at200.hex spent_as_unspent.hex "import struct; sl=struct.unpack_from('<H',b,59)[0]; b[5]=0; del b[61+sl:73+sl]"
case_ "spent coin re-encoded as unspent"              1 --liveness "$T/spent_as_unspent.hex" "$T9:0@200" "$R"
tamper b170_0_at200.hex value.hex "b[43]^=1"
case_ "amount changed in the proof"                   1 --liveness "$T/value.hex" "$T170:0@200" "$R"
tamper b170_0_at200.hex never.hex "b[7]^=0x80"
case_ "an outpoint that never existed (txid changed)" 1 --liveness "$T/never.hex" "$(python3 -c "t=bytearray(bytes.fromhex('$T170'))[::-1];t[0]^=0x80;print(t[::-1].hex())"):0@200" "$R"
tamper b170_0_at200.hex above.hex "import struct; sl=struct.unpack_from('<H',b,59)[0]; struct.pack_into('<I',b,61+sl,201)"
case_ "a height above the proof's tip"                1 --liveness "$T/above.hex" "$T170:0@201" "$R"
tamper b170_0_at200.hex path.hex "b[-1]^=1"
case_ "tampered witness (log path)"                   1 --liveness "$T/path.hex" "$T170:0@200" "$R"
tamper b9_0_at169.hex bpath.hex "b[-40]^=1"
case_ "tampered witness (block path)"                 1 --liveness "$T/bpath.hex" "$T9:0@169" "$R"
tamper b170_0_at200.hex trailing.hex "b.append(0)"
case_ "a byte appended"                               1 --liveness "$T/trailing.hex" "$T170:0@200" "$R"
case_ "dev-mode receipt (FakeReceipt)"                1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$C/fake_block_2501.rcpt"
case_ "a receipt with one seal bit flipped"           1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$C/mainnet_1_200_8888d59c_flipped.rcpt"
if true; then
  # the producer: the same bytes again, and no proof for an outpoint that never existed
  B=$D/../../../prover/testdata/libzync/mainnet
  "$V" prove "$D/mainnet_1_200_history.bin" 200 tip "$T9:0" --blocks "$B" > "$T/p.hex" 2>/dev/null
  if cmp -s <(tr -d '\n' < "$T/p.hex") <(tr -d '\n' < "$D/b9_0_at200.hex"); then pass=$((pass+1)); echo "ok   zapp-liveness prove reproduces b9_0_at200.hex"
  else fail=$((fail+1)); echo "FAIL zapp-liveness prove differs"; fi
  if "$V" prove "$D/mainnet_1_200_history.bin" 200 tip "$T170:2" --blocks "$B" > /dev/null 2>&1; then fail=$((fail+1)); echo "FAIL zapp-liveness prove made a proof for a missing output"
  else pass=$((pass+1)); echo "ok   zapp-liveness prove refuses an output that does not exist"; fi
fi
echo "LIVENESS_CLI: $pass passed, $fail failed"
[ "$fail" = 0 ]
