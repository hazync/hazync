#!/bin/bash
# Liveness proofs through the liveness zAPP's CLI (docs/libzync/LIVENESS_ZAPP.md), against the engine chain's real
# proof of mainnet 1..200 (the core's swiftsync-verify/vectors, under its image id and key; the proofs in engine/), and
# its producer (zapp-liveness prove) on that run's history (node format 2).
# usage: run.sh <zapp-liveness>     exit codes: 2 DEMO (honest: below minimum work), 1 FAIL
set -u
V=${1:?zapp-liveness}
D=$(cd "$(dirname "$0")/engine" && pwd); C=$D/../../../../swiftsync-verify/vectors; R=$C/mainnet_1_200_engine.hze
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
IMG="--image $(cat $C/mainnet_1_200_engine.image_id) --vk $C/mainnet_1_200_engine.vk"
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
case_ "created= pins the coin"                        2 --liveness "$D/b9_0_at200.hex"   "$T9:0@200=spent,created=9,coinbase=1" "$R"
case_ "another creation height asked"                 1 --liveness "$D/b9_0_at200.hex"   "$T9:0@200=spent,created=10" "$R"
case_ "coinbase flag asked wrongly"                   1 --liveness "$D/b9_0_at200.hex"   "$T9:0@200,coinbase=0" "$R"
case_ "upper-case txid in the question"               1 --liveness "$D/b170_0_at200.hex" "$(echo $T170 | tr a-f A-F):0@200" "$R"
case_ "leading zero in the question"                  1 --liveness "$D/b170_0_at200.hex" "$T170:00@200" "$R"
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
# the image id and key (no engine core image is published yet): none, another id, a key that is not the image's
IMG0=$IMG; IMG=""
case_ "no --image / --vk"                              1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$R"
IMG="--image $(printf 'ab%.0s' $(seq 32)) --vk $C/mainnet_1_200_engine.vk"
case_ "another core image id"                          1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$R"
IMG="--image $(cat $C/mainnet_1_200_engine.image_id) --vk $C/mainnet_1_200_engine_key_flipped.vk"
case_ "a key that is not the image's"                  1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$R"
IMG=$IMG0
case_ "a proof with one bit flipped"                   1 --liveness "$D/b170_0_at200.hex" "$T170:0@200" "$C/mainnet_1_200_engine_proof_flipped.hze"
# the chain identity through the other sources (zapp-common, every zAPP CLI's flags): a node that agrees (hash and
# chainwork), one that names another block, and an --expect it contradicts
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\n[ "$1" = getblockheader ] && echo "{\\"chainwork\\": \\"%s\\"}"\nexit 0\n' 000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320 "$(printf '%054d' 0)c900c900c9" > "$T/node"; chmod +x "$T/node"
printf '#!/bin/sh\necho %064d\n' 1 > "$T/badnode"; chmod +x "$T/badnode"
case_ "your node agrees (hash and chainwork)"         2 --bitcoin-cli "$T/node" --liveness "$D/b9_0_at200.hex" "$T9:0@200=spent:170:1:0" "$R"
case_ "your node names another block"                 1 --bitcoin-cli "$T/badnode" --liveness "$D/b9_0_at200.hex" "$T9:0@200=spent:170:1:0" "$R"
case_ "an --expect your node contradicts"             1 --bitcoin-cli "$T/node" --expect "200:$(printf '%064d' 0)" --liveness "$D/b9_0_at200.hex" "$T9:0@200" "$R"
if true; then
  # the producer: the same bytes again, and no proof for an outpoint that never existed
  B=$D/../../../../prover/testdata/libzync/mainnet; HIST=$C/../tests/data/engine200/history.bin
  # the engine chain's history: node format 2 (the default)
  "$V" prove "$HIST" 200 tip "$T9:0" --blocks "$B" > "$T/p.hex" 2>/dev/null
  if cmp -s <(tr -d '\n' < "$T/p.hex") <(tr -d '\n' < "$D/b9_0_at200.hex"); then pass=$((pass+1)); echo "ok   zapp-liveness prove reproduces b9_0_at200.hex"
  else fail=$((fail+1)); echo "FAIL zapp-liveness prove differs"; fi
  if "$V" prove "$HIST" 200 tip "$T170:2" --blocks "$B" > /dev/null 2>&1; then fail=$((fail+1)); echo "FAIL zapp-liveness prove made a proof for a missing output"
  else pass=$((pass+1)); echo "ok   zapp-liveness prove refuses an output that does not exist"; fi
fi
echo "LIVENESS_CLI: $pass passed, $fail failed"
[ "$fail" = 0 ]
