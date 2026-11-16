#!/bin/bash
# zapp-assume's OK verdict on REAL proofs: the zAPP regtest harness's GPU receipts (scripts/zapp-regtest/receipts, image
# eb2cd424, a 104/105/110/112 and b 113) with Core's oracle (scripts/zapp-regtest/oracle: dumptxoutset files,
# gettxoutsetinfo hash_serialized_3, block hashes) as the independent answer.
#   test-regtest.sh <zapp-assume built with --features regtest-test (dev profile)> <the shipped zapp-assume (--release)>
# Writes zapps/assume/test-regtest.out. Every regtest report must start "TEST IMAGE (regtest-test)". The headers files
# are made from Core's block headers (oracle_node.py, each hash checked against Core's); the BIP30-tracked list is the
# harness's one tracked coinbase (block 2's, repeated at 105: chain/scenarios.json tracked_coinbase).
V=$(realpath "$1"); S=$(realpath "$2")
cd "$(dirname "$0")/../.." || exit 1
exec > zapps/assume/test-regtest.out 2>&1
BAD=0; N=0
G=scripts/zapp-regtest; O="python3 $G/oracle_node.py"
T=$(mktemp -d); trap 'rm -rf $T' EXIT
hash_at() { $O "$1" getblockhash "$2"; }
h3() { python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['gettxoutsetinfo_hash_serialized_3']['hash_serialized_3'])" "$G/oracle/snap_$1.json"; }
A112=$(hash_at a 112); B112=$(hash_at b 112); B113=$(hash_at b 113); B105=$(hash_at a 105)
$O a headers $T/ha.bin 2>/dev/null; $O b headers $T/hb.bin 2>/dev/null
grep -o '"live_coin": "[0-9a-f]*:[0-9]*"' $G/chain/scenarios.json | cut -d'"' -f4 > $T/tracked.txt
echo "tracked: $(cat $T/tracked.txt)"
run() { local want=$1; local grepfor=$2; shift 2; echo "== $* (expect exit $want)" | sed "s#$T#<tmp>#g; s#$V#zapp-assume[regtest-test]#; s#$S#zapp-assume[shipped]#"; local out; out=$("$@" 2>&1); local got=$?
        echo "$out" | grep -a 'VERIFY_\|\[FAIL\]\|TEST IMAGE\|assumeutxo:.*proven UTXO\|assumevalid: -assumevalid' | sed "s#$T#<tmp>#g"
        N=$((N+1))
        # every report of the feature build (all on the regtest image) carries the TEST IMAGE line; the shipped one never
        local marked=1 tl; tl=$(echo "$out" | grep -ac '^  \[ ok \] TEST IMAGE (regtest-test): never a release' || true)
        if [ "$1" = "$V" ]; then [ "$tl" = 1 ] || marked=0; else [ "$tl" = 0 ] || marked=0; fi
        if [ "$got" = "$want" ] && [ "$marked" = 1 ] && { [ -z "$grepfor" ] || echo "$out" | grep -aq -- "$grepfor"; }; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
U="$V assumeutxo --network regtest --tracked $T/tracked.txt"
# ---- OK: every kept receipt, its own height's dumptxoutset, Core's hash_serialized_3 printed back
for x in a:104 a:105 a:110 a:112 b:113; do
  b=${x%:*} h=${x#*:}; lab=$h; [ "$h" -ge 112 ] && lab=$b$h
  head -c $(( (h + 1) * 80 )) $T/h$b.bin > $T/h_$lab.bin
  run 0 "is the proven UTXO set at height $h; Core's hash_serialized_3 of it: $(h3 $lab)" \
      $U --headers $T/h_$lab.bin --expect $h:$(hash_at $b $h) $G/receipts/$b/block_$h.rcpt $G/oracle/utxo_$lab.dat
done
# the chain identity from a node (oracle_node.py, Core's answers): on branch a (after invalidateblock b 111) OK; on the
# active chain after the reorg (b) block 112 is b's: FAIL
run 0 "is the proven UTXO set at height 112" $U --headers $T/ha.bin --bitcoin-cli "$O a -regtest" $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
run 1 "block 112 is $B112" $U --headers $T/ha.bin --bitcoin-cli "$O b -regtest" $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
run 0 "is the proven UTXO set at height 113" $U --headers $T/hb.bin --bitcoin-cli "$O b -regtest" $G/receipts/b/block_113.rcpt $G/oracle/utxo_b113.dat
# the reorg through --expect: b's 112, b's tip
run 1 "block 112 is $B112" $U --headers $T/ha.bin --expect 112:$B112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
run 1 "block 113 is $B113" $U --headers $T/ha.bin --expect 113:$B113 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
# another branch's snapshot, another height's, no tracked list (the live repeated coinbase is tracked): FAIL
run 1 "not the proven tip" $U --headers $T/ha.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_b113.dat
run 1 "not the proven tip" $U --headers $T/ha.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_110.dat
run 1 "NOT the proven UTXO set" $V assumeutxo --network regtest --headers $T/ha.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
# b's headers 0..112 for a's receipt (b's block 112 last)
head -c $((113 * 80)) $T/hb.bin > $T/hb112.bin
run 1 "the last header hashes to $B112, not the proven tip" $U --headers $T/hb112.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
# no identity source: DEMO, not OK
run 2 "facts need the tip compared" $U --headers $T/ha.bin $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
# ---- assumevalid: the tip, and block 105 through the headers
run 0 "assumevalid=$A112" $V assumevalid --network regtest --expect 112:$A112 $G/receipts/a/block_112.rcpt
run 0 "assumevalid=$B105 (height 105" $V assumevalid --network regtest --block $B105 --headers $T/ha.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt
run 1 "not in the headers" $V assumevalid --network regtest --block $B112 --headers $T/ha.bin --expect 112:$A112 $G/receipts/a/block_112.rcpt
run 0 "assumevalid=$B113" $V assumevalid --network regtest --expect 113:$B113 $G/receipts/b/block_113.rcpt
# ---- positive controls: the SHIPPED CLI (--release, no feature) refuses every regtest receipt
run 1 "unknown network" $S assumeutxo --network regtest --headers $T/ha.bin --tracked $T/tracked.txt --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
run 1 "receipt verifies against image" $S assumeutxo --headers $T/ha.bin --tracked $T/tracked.txt --expect 112:$A112 $G/receipts/a/block_112.rcpt $G/oracle/utxo_a112.dat
run 1 "receipt verifies against image" $S assumevalid --network signet --expect 112:$A112 $G/receipts/a/block_112.rcpt
# the regtest image's id named on the shipped CLI: mainnet's parameters re-pointed at it, so the profile and the anchor
# are mainnet's: FAIL
run 1 "profile 0x79 == 0x71" $S assumevalid --image eb2cd424b94a4aa74f3458efc0d5edb1d3c35d46615eab8363696f1fa4b47ccd --expect 112:$A112 $G/receipts/a/block_112.rcpt
echo "zapp-assume test-regtest.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
