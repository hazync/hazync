#!/usr/bin/env bash
# The header-chain zAPP's CLI (zapp-headers) on the engine's REAL proof of mainnet headers 0..4,031 (tests/data/engine4031:
# keys-hdr, proved on CPU 26.11.2026) and every way of misusing it, and the MMR inclusion checks (block 200's path to
# the tip 4,031, mmr_path_200_tip4031.txt). Engine proofs only since the cutover (27.11.2026); the RISC Zero receipts'
# cases (two chained runs, block 170's transaction through --tx) are v0.27.1's. Usage: test_headers_verify.sh <zapp-headers>
# the key pin a reader gives with every key (review R1-6 #1: a key is never pinned to itself); here the fixture key's own
# SHA-256 stands for "your own build's vk.sha256"
pin() { sha256sum "$1" | cut -c1-64; }
set -u
V=${1:?zapp-headers binary}; D=$(cd "$(dirname "$0")/tests/data/engine4031" && pwd)
ID=$(cat "$D/image_id"); K=(--image "$ID" --vk "$D/vk.bin" --vk-sha256 "$(pin "$D/vk.bin")")
TIP=00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf   # mainnet block 4,031
H200=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
python3 -c "import sys;b=bytearray(open(sys.argv[1],'rb').read());b[len(b)//2]^=1;open(sys.argv[2],'wb').write(b)" "$D/proof.hze" "$T/bad.hze"
python3 -c "import sys;b=bytearray(open(sys.argv[1],'rb').read());b[1000]^=1;open(sys.argv[2],'wb').write(b)" "$D/vk.bin" "$T/badkey.bin"
fails=0
t() { local want=$1 name=$2; shift 2; "$V" "$@" >"$T/out" 2>&1; local rc=$?
      if [ "$rc" = "$want" ]; then echo "ok   $name (exit $rc)"; else echo "FAIL $name: exit $rc, want $want"; tail -3 "$T/out"; fails=$((fails+1)); fi; }
P=$D/proof.hze
t 2 "genesis..4,031, tip expected: DEMO (below minimum work)" "${K[@]}" --expect 4031:$TIP "$P"
t 2 "genesis..4,031 alone" "${K[@]}" "$P"
t 1 "the same run twice" "${K[@]}" "$P" "$P"
t 1 "one bit flipped in the proof" "${K[@]}" "$T/bad.hze"
t 1 "wrong expected tip hash" "${K[@]}" --expect 4031:${TIP%?}e "$P"
t 1 "wrong expected tip height" "${K[@]}" --expect 4030:$TIP "$P"
t 1 "another image id" --image 3f5030006310997a9f63b7f9a59c41ebbd99035d2db2a0f062ceb12f2615bee7 --vk "$D/vk.bin" --vk-sha256 "$(pin "$D/vk.bin")" "$P"
t 1 "a key with byte 1000 changed (its last ~8 bytes do not affect verification: the pin binds them)" --image "$ID" --vk "$T/badkey.bin" --vk-sha256 "$(pin "$T/badkey.bin")" "$P"
t 1 "no --image (no engine image is published yet)" --vk "$D/vk.bin" --vk-sha256 "$(pin "$D/vk.bin")" "$P"
t 1 "no --vk" --image "$ID" "$P"
t 1 "the key without --vk-sha256 (never pinned to itself: review R1-6 #1)" --image "$ID" --vk "$D/vk.bin" "$P"
t 1 "the key with a pin that is not its SHA-256" --image "$ID" --vk "$D/vk.bin" --vk-sha256 0000000000000000000000000000000000000000000000000000000000000000 "$P"
t 1 "not a proof" "${K[@]}" "$D/image_id"
# MMR inclusion (block 200 = leaf 200, at the tip 4,031)
PA="$D/mmr_path_200_tip4031.txt"
head -n -1 "$PA" > "$T/short.txt"; { cat "$PA"; tail -1 "$PA"; } > "$T/long.txt"
t 2 "block 200 is on the chain (MMR path)" "${K[@]}" --expect 4031:$TIP --include 200:$H200 --path "$PA" "$P"
t 1 "block 200's path at height 201" "${K[@]}" --include 201:$H200 --path "$PA" "$P"
t 1 "another hash on block 200's path" "${K[@]}" --include 200:${H200%?}1 --path "$PA" "$P"
t 1 "a truncated path" "${K[@]}" --include 200:$H200 --path "$T/short.txt" "$P"
t 1 "an extended path" "${K[@]}" --include 200:$H200 --path "$T/long.txt" "$P"
t 1 "a block above the tip" "${K[@]}" --include 4032:$H200 --path "$PA" "$P"
t 1 "--include without --path" "${K[@]}" --include 200:$H200 "$P"
echo "zapp-headers CLI: $fails failure(s)"
[ "$fails" = 0 ]
