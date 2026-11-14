#!/usr/bin/env bash
# The header-chain verifier (swiftsync-verify headers) on two REAL chained receipts (CPU-proved, 11-10: genesis..100,
# then 101..200 started from the first one's end state) and every way of misusing them, and the MMR inclusion checks
# (block 37's path to the tip 200, mmr_path_37_tip200.txt; transaction f4184fc5... at position 1 of block 170 by its
# header and a one-hash branch, branch_170_1.txt). Usage: test_headers_verify.sh <swiftsync-verify>
# HEADERS_IMAGE=<hex>: verify against this image id instead of the built-in one (receipts proved by a local build).
set -u
V=${1:?swiftsync-verify binary}; D=$(cd "$(dirname "$0")" && pwd)
IMG=(); [ -n "${HEADERS_IMAGE:-}" ] && IMG=(--image "$HEADERS_IMAGE")
H=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320   # mainnet block 200
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
python3 -c "import sys;b=bytearray(open(sys.argv[1],'rb').read());b[len(b)//2]^=1;open(sys.argv[2],'wb').write(b)" "$D/hdr_101_200.rcpt" "$T/bad.rcpt"
fails=0
t() { local want=$1 name=$2; shift 2; "$V" headers "${IMG[@]}" "$@" >"$T/out" 2>&1; local rc=$?
      if [ "$rc" = "$want" ]; then echo "ok   $name (exit $rc)"; else echo "FAIL $name: exit $rc, want $want"; tail -3 "$T/out"; fails=$((fails+1)); fi; }
t 2 "genesis..100 + 101..200, tip expected: DEMO (below minimum work)" --expect 200:$H "$D/hdr_0_100.rcpt" "$D/hdr_101_200.rcpt"
t 2 "genesis..100 alone" "$D/hdr_0_100.rcpt"
t 1 "wrong order" "$D/hdr_101_200.rcpt" "$D/hdr_0_100.rcpt"
t 1 "a run that does not start from genesis" "$D/hdr_101_200.rcpt"
t 1 "the same run twice" "$D/hdr_0_100.rcpt" "$D/hdr_0_100.rcpt"
t 1 "one bit flipped in a receipt" "$D/hdr_0_100.rcpt" "$T/bad.rcpt"
t 1 "wrong expected tip hash" --expect 200:${H%?}1 "$D/hdr_0_100.rcpt" "$D/hdr_101_200.rcpt"
t 1 "wrong expected tip height" --expect 199:$H "$D/hdr_0_100.rcpt" "$D/hdr_101_200.rcpt"
t 1 "another image id" --image 3f5030006310997a9f63b7f9a59c41ebbd99035d2db2a0f062ceb12f2615bee7 "$D/hdr_0_100.rcpt"
t 1 "not a receipt" "$D/h0_200.txt"
# MMR inclusion (block 37 = leaf 37, inside the 128-block peak of the 201-block chain: 7 hashes)
H37=00000000ddd96d128e122d1179034ff66c27dc583eb9e8996a0b1779c60c6f86
H38=000000007c19ee8e924d3024d58efff50f872aadf256140bb1e3d62fea4fd6dd
P="$D/mmr_path_37_tip200.txt"; R=("$D/hdr_0_100.rcpt" "$D/hdr_101_200.rcpt")
head -6 "$P" > "$T/short.txt"; { cat "$P"; tail -1 "$P"; } > "$T/long.txt"
t 2 "block 37 is on the chain (MMR path)" --expect 200:$H --include 37:$H37 --path "$P" "${R[@]}"
t 1 "block 37's path at height 38" --include 38:$H37 --path "$P" "${R[@]}"
t 1 "block 38's hash on block 37's path" --include 37:$H38 --path "$P" "${R[@]}"
t 1 "a truncated path" --include 37:$H37 --path "$T/short.txt" "${R[@]}"
t 1 "an extended path" --include 37:$H37 --path "$T/long.txt" "${R[@]}"
t 1 "block 37's path against the chain to 100 only (another MMR)" --include 37:$H37 --path "$P" "$D/hdr_0_100.rcpt"
t 1 "a block above the tip" --include 201:$H37 --path "$P" "${R[@]}"
# a transaction in block 170 (Satoshi to Hal Finney), by its header and merkle branch (the coinbase txid)
H170=00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee
HDR170=$(awk '$1==170 {print $2}' "$D/h0_200.txt")
TX=f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
t 2 "tx $TX is in block 170 on the chain" --include 170:$H170 --path "$D/mmr_path_170_tip200.txt" --tx $TX:1 --header $HDR170 --branch "$D/branch_170_1.txt" "${R[@]}"
t 1 "  at the wrong position" --include 170:$H170 --path "$D/mmr_path_170_tip200.txt" --tx $TX:0 --header $HDR170 --branch "$D/branch_170_1.txt" "${R[@]}"
t 1 "  under another block's header" --include 170:$H170 --path "$D/mmr_path_170_tip200.txt" --tx $TX:1 --header "$(awk '$1==171 {print $2}' "$D/h0_200.txt")" --branch "$D/branch_170_1.txt" "${R[@]}"
t 1 "  a txid that is not in it" --include 170:$H170 --path "$D/mmr_path_170_tip200.txt" --tx ${TX%?}7:1 --header $HDR170 --branch "$D/branch_170_1.txt" "${R[@]}"
[ $fails = 0 ] && echo "HEADERS_VERIFY_TESTS_OK" || { echo "HEADERS_VERIFY_TESTS: $fails FAILED"; exit 1; }
