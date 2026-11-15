#!/bin/bash
# swiftsync-verify assumevalid / assumeutxo end-to-end (docs/libzync/ASSUME_ZAPPS.md): the real GPU proof of mainnet
# 1..200 (8888d59c), Core v31.1.0 dumptxoutset files at 199, 200 and 1003 and the headers 0..200 (vectors/assume/),
# and corrupted copies. Run from the repo root after `cargo build --release`. Writes swiftsync-verify/test-assume.out.
# Every case prints its verdict line and exit code. A proof at 200 is below Core's minimum chainwork, so a passing
# check is DEMO (exit 2), never OK; the line `... is the proven UTXO set` / `-assumevalid=` says the check itself held.
cd "$(dirname "$0")/.." || exit 1
exec > swiftsync-verify/test-assume.out 2>&1
BAD=0; N=0
V=swiftsync-verify/target/release/swiftsync-verify; A=swiftsync-verify/vectors/assume
R=swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt; F=swiftsync-verify/vectors/mainnet_1_200_8888d59c_flipped.rcpt
TIP=000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
B170=00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee
B1003=$(python3 -c "import sys; print(open(sys.argv[1],'rb').read()[11:43][::-1].hex())" $A/utxo_1003.dat)
T=$(mktemp -d); trap 'rm -rf $T' EXIT
# corrupted copies: byte offsets of Core's snapshot header (magic 0-4, version 5-6, network 7-10, base 11-42, count 43-50, first txid 51-82)
flip() { python3 -c "import sys; b=bytearray(open(sys.argv[1],'rb').read()); b[int(sys.argv[2])]^=1; open(sys.argv[3],'wb').write(b)" "$@"; }
flip $A/utxo_200.dat 51 $T/txid.dat; flip $A/utxo_200.dat 8 $T/net.dat; flip $A/utxo_200.dat 20 $T/base.dat
flip $A/utxo_200.dat 200 $T/body.dat
cp $A/utxo_200.dat $T/trail.dat; printf '\0' >> $T/trail.dat
head -c 14269 $A/utxo_200.dat > $T/short.dat
head -c 16000 $A/headers_0_200.bin > $T/hdr_short.bin
flip $A/headers_0_200.bin $((80*150+68)) $T/hdr_time.bin
tail -c $((80*51)) $A/headers_0_200.bin > $T/hdr_150.bin
# the coinbase of block 1 (unspent at 200) marked BIP30-tracked: a false side input
echo "0e3e2357e806b6cdb1f70b54c3a3a17b6714ee1f0e68bebb44a74b1efd512098:0" > $T/tracked.txt
run() { local want=$1; local grepfor=$2; shift 2; echo "== $* (expect exit $want)"; local out; out=$("$@" 2>&1); local got=$?
        echo "$out" | grep -a 'VERIFY_\|\[FAIL\]\|assumeutxo:.*proven UTXO\|assumevalid: -assumevalid' | sed "s#$T#<tmp>#g"
        N=$((N+1)); if [ "$got" = "$want" ] && { [ -z "$grepfor" ] || echo "$out" | grep -aq -- "$grepfor"; }; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
U="$V assumeutxo --headers $A/headers_0_200.bin"
run 2 "is the proven UTXO set at height 200; Core's hash_serialized_3 of it: a51f51ae" $U --expect 200:$TIP $R $A/utxo_200.dat
run 2 "is the proven UTXO set" $U $R $A/utxo_200.dat
run 1 "" $U --expect 200:$B170 $R $A/utxo_200.dat
run 1 "" $U $F $A/utxo_200.dat
run 1 "NOT the proven UTXO set" $U $R $T/txid.dat
run 1 "" $U $R $T/body.dat
run 1 "network magic" $U $R $T/net.dat
run 1 "is not the proven tip" $U $R $T/base.dat
run 1 "bytes after" $U $R $T/trail.dat
run 1 "truncated" $U $R $T/short.dat
run 1 "tip only" $U $R $A/utxo_199.dat
run 1 "is not the proven tip" $U $R $A/utxo_1003.dat
run 1 "NOT the proven UTXO set" $U --tracked $T/tracked.txt $R $A/utxo_200.dat
run 1 "required" $V assumeutxo $R $A/utxo_200.dat
run 1 "more than\|not a positive\|not the proven tip" $V assumeutxo --headers $T/hdr_short.bin $R $A/utxo_200.dat
run 1 "does not link" $V assumeutxo --headers $T/hdr_time.bin $R $A/utxo_200.dat
run 1 "from genesis" $V assumeutxo --headers $T/hdr_150.bin $R $A/utxo_200.dat
run 1 "" $U $R /nonexistent.dat
run 2 "assumevalid=$TIP" $V assumevalid --expect 200:$TIP $R
run 2 "assumevalid=$B170 (height 170" $V assumevalid --block $B170 --headers $A/headers_0_200.bin --expect 200:$TIP $R
run 2 "assumevalid=$B170 (height 170" $V assumevalid --block $B170 --headers $T/hdr_150.bin $R
run 1 "not in the headers" $V assumevalid --block $B1003 --headers $A/headers_0_200.bin $R
run 1 "give the headers" $V assumevalid --block $B170 $R
run 1 "" $V assumevalid --block $B170 --headers $A/headers_0_200.bin $F
run 1 "not 64 hex" $V assumevalid --block xyz $R
run 1 "does not link" $V assumevalid --block $B170 --headers $T/hdr_time.bin $R
# the independent reference (scripts/libzync/assume_ref.py) must print the hash_serialized_3 and live hash the verifier printed
echo "== assume_ref.py == the verifier's live_hash and hash_serialized_3"
REF=$(python3 -I scripts/libzync/assume_ref.py $A/utxo_200.dat $A/headers_0_200.bin); echo "$REF"
OUT=$($U $R $A/utxo_200.dat)
N=$((N+1)); if [ -n "$REF" ] && echo "$OUT" | grep -aqF "$(echo "$REF" | grep live_hash)" && echo "$OUT" | grep -aqF "$(echo "$REF" | grep hash_serialized_3 | cut -d' ' -f2)" \
   && echo "$REF" | grep -q "^hash_serialized_3 a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67$"; then echo "PASS"; else echo "WRONG"; BAD=$((BAD+1)); fi
echo "swiftsync-verify test-assume.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
