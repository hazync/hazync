#!/usr/bin/env bash
# The header-chain verifier (swiftsync-verify headers) on two REAL chained receipts (CPU-proved, 11-10: genesis..100,
# then 101..200 started from the first one's end state) and every way of misusing them. Usage: test_headers_verify.sh <swiftsync-verify>
set -u
V=${1:?swiftsync-verify binary}; D=$(cd "$(dirname "$0")" && pwd)
H=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320   # mainnet block 200
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
python3 -c "import sys;b=bytearray(open(sys.argv[1],'rb').read());b[len(b)//2]^=1;open(sys.argv[2],'wb').write(b)" "$D/hdr_101_200.rcpt" "$T/bad.rcpt"
fails=0
t() { local want=$1 name=$2; shift 2; "$V" headers "$@" >"$T/out" 2>&1; local rc=$?
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
[ $fails = 0 ] && echo "HEADERS_VERIFY_TESTS_OK" || { echo "HEADERS_VERIFY_TESTS: $fails FAILED"; exit 1; }
