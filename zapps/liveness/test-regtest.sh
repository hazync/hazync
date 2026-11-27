#!/bin/bash
# zapp-liveness's OK verdict on REAL proofs: the zAPP regtest harness's GPU receipts (scripts/zapp-regtest/receipts,
# image eb2cd424: a 112, b 113) and each branch's history.bin, with Core's oracle (scripts/zapp-regtest/oracle) as the
# independent answer: every claim checked below is made from Core's data alone (gettxout with include_mempool=false
# at the branch's tip or at 110 for a coin's value, script, creation height and coinbase flag; the spending
# transaction's position and input from Core's block), never from the zAPP.
#   test-regtest.sh <zapp-liveness built with --features regtest-test (dev profile)> <the shipped zapp-liveness (--release)>
# Writes zapps/liveness/test-regtest.out. Every report of the feature build starts "TEST IMAGE (regtest-test)".
# The reorg (README, chain/scenarios.json "reorg"): X is spent in a 111 and unspent on b; Y unspent on a, spent in b 111'.
V=$(realpath "$1"); S=$(realpath "$2")
cd "$(dirname "$0")/../.." || exit 1
exec > zapps/liveness/test-regtest.out 2>&1
BAD=0; N=0
G=scripts/zapp-regtest; O="python3 $G/oracle_node.py"
T=$(mktemp -d); trap 'rm -rf $T' EXIT
A112=$($O a getblockhash 112); B112=$($O b getblockhash 112); B113=$($O b getblockhash 113)
# each branch's block files in one directory, as the prover read them (blocks/main + blocks/<branch>)
for b in a b; do mkdir -p $T/blocks_$b; cp $G/blocks/main/*.json $G/blocks/$b/*.json $T/blocks_$b/; done
X=$(python3 -c "import json; print(json.load(open('$G/chain/scenarios.json'))['scenarios']['reorg']['X']['outpoint'])")
Y=$(python3 -c "import json; print(json.load(open('$G/chain/scenarios.json'))['scenarios']['reorg']['Y']['outpoint'])")
RC=$(python3 -c "import json; print(json.load(open('$G/chain/scenarios.json'))['scenarios']['repeated_coinbase_105']['live_coin'])")
# ALICE's P2SH-P2WPKH coin of block 102 (output 4), spent at 106 (scenarios varied_106_110)
L1=$(python3 -c "import json; print(json.load(open('$G/chain/scenarios.json'))['scenarios']['spend_types_102']['txid'])"):4
# claim <branch> <outpoint> <snapshot with the coin unspent>: Core's answer for the outpoint on that branch's tip,
# "=unspent" or "=spent:<h>:<tx index>:<vin>", and the coin's value, script, creation height and coinbase flag
claim() { python3 - "$G" "$@" <<'P'
import json, sys
g, br, op, snap = sys.argv[1:]
c = json.load(open(f"{g}/oracle/snap_{snap}.json"))["gettxout"][op]
tip = json.load(open(f"{g}/oracle/snap_{'a112' if br == 'a' else 'b113'}.json"))["gettxout"].get(op)
s = "=unspent"
if tip is None:
    for src in ("main", br):
        for h, blk in json.load(open(f"{g}/oracle/blocks_{src}.json")).items():
            for sp in blk["spends"]:
                if sp["prev"] == op:
                    s = f"=spent:{h}:{blk['txids'].index(sp['txid'])}:{sp['vin']}"
print(f"{s},value={c['value_sat']},script={c['spk']},created={c['height']},coinbase={int(c['coinbase'])}")
P
}
# the harness's receipts are RISC Zero proofs: coin log node format 1
prove() { $V prove $G/receipts/$1/history.bin $2 $3 $4 --blocks $T/blocks_$1 --node-format 1 > $T/$5.hex || { echo "prove $*: FAILED"; BAD=$((BAD+1)); }; }
prove a 112 tip $X xa; prove b 113 tip $X xb; prove a 112 tip $Y ya; prove b 113 tip $Y yb
prove a 112 110 $X xa110; prove a 112 tip $RC rca; prove a 112 tip $L1 l1a
# the committed vector (the WASM and C ABI controls read it) is what the prover makes from the committed history
N=$((N+1)); if cmp -s $T/xa.hex zapps/liveness/vectors/regtest_x_spent_a112.hex; then echo "vectors/regtest_x_spent_a112.hex = prove a 112 X: PASS"
else echo "vectors/regtest_x_spent_a112.hex != prove a 112 X: WRONG"; BAD=$((BAD+1)); fi
CXA=$(claim a $X 110); CXB=$(claim b $X b113); CYA=$(claim a $Y a112); CYB=$(claim b $Y 110); CRC=$(claim a $RC a112); CL1=$(claim a $L1 105)
echo "Core's answers: X on a $CXA; X on b $CXB; Y on a $CYA; Y on b $CYB; block-105 coinbase on a $CRC; ALICE P2SH-P2WPKH on a $CL1"
run() { local want=$1; local grepfor=$2; shift 2
        echo "== $* (expect exit $want)" | sed "s#$T#<tmp>#g; s#$V#zapp-liveness[regtest-test]#; s#$S#zapp-liveness[shipped]#"
        local out; out=$("$@" 2>&1); local got=$?
        echo "$out" | grep -a 'VERIFY_\|\[FAIL\]\|TEST IMAGE\|--liveness' | sed "s#$T#<tmp>#g"
        N=$((N+1))
        # every report of the feature build (all on the regtest image) carries the TEST IMAGE line; the shipped one never
        local marked=1 tl; tl=$(echo "$out" | grep -ac '^  \[ ok \] TEST IMAGE (regtest-test): never a release' || true)
        if [ "$1" = "$V" ]; then [ "$tl" = 1 ] || marked=0; else [ "$tl" = 0 ] || marked=0; fi
        if [ "$got" = "$want" ] && [ "$marked" = 1 ] && { [ -z "$grepfor" ] || echo "$out" | grep -aq -- "$grepfor"; }; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
L="$V --network regtest"
RA=$G/receipts/a/block_112.rcpt; RB=$G/receipts/b/block_113.rcpt
# ---- OK: Core's answer, proven, on each branch
run 0 "is SPENT at height 111" $L --expect 112:$A112 --liveness $T/xa.hex "$X@112$CXA" $RA
run 0 "is UNSPENT" $L --expect 113:$B113 --liveness $T/xb.hex "$X@113$CXB" $RB
run 0 "is UNSPENT" $L --expect 112:$A112 --liveness $T/ya.hex "$Y@112$CYA" $RA
run 0 "is SPENT at height 111" $L --expect 113:$B113 --liveness $T/yb.hex "$Y@113$CYB" $RB
run 0 "SWIFTSYNC_VERIFY_OK: 4 liveness" $L --expect 112:$A112 --liveness $T/xa.hex "$X@112$CXA" --liveness $T/ya.hex "$Y@112$CYA" \
    --liveness $T/rca.hex "$RC@112$CRC" --liveness $T/l1a.hex "$L1@112$CL1" $RA
# X at 110 (before a's spend), on the 112 proof: unspent then
run 0 "is UNSPENT" $L --expect 112:$A112 --liveness $T/xa110.hex "$X@110=unspent" $RA
# the identity from a node (Core's answers, oracle_node.py): on a (after invalidateblock b 111) OK; the active chain after
# the reorg names b's block 112: FAIL
run 0 "is SPENT at height 111" $L --bitcoin-cli "$O a -regtest" --liveness $T/xa.hex "$X@112$CXA" $RA
run 1 "says block 112 is $B112\|block 112 is $B112" $L --bitcoin-cli "$O b -regtest" --liveness $T/xa.hex "$X@112$CXA" $RA
run 0 "is UNSPENT" $L --bitcoin-cli "$O b -regtest" --liveness $T/xb.hex "$X@113$CXB" $RB
# ---- the reorg through --expect: a's proof is OK at a 112, FAIL with b's 112 or b's tip
run 1 "not the proof's tip" $L --expect 113:$B113 --liveness $T/xa.hex "$X@112$CXA" $RA
run 1 "not the proof's tip" $L --expect 112:$B112 --liveness $T/xa.hex "$X@112$CXA" $RA
# ---- wrong claims (the other branch's answer) and a proof from the other branch's history: FAIL
run 1 "" $L --expect 112:$A112 --liveness $T/xa.hex "$X@112=unspent" $RA
run 1 "" $L --expect 113:$B113 --liveness $T/yb.hex "$Y@113=unspent" $RB
run 1 "" $L --expect 112:$A112 --liveness $T/xb.hex "$X@112$CXA" $RA
run 1 "" $L --expect 112:$A112 --liveness $T/xa.hex "$X@112${CXA/value=/value=1}" $RA
# no identity source: DEMO, not OK
run 2 "SWIFTSYNC_VERIFY_DEMO" $L --liveness $T/xa.hex "$X@112$CXA" $RA
# ---- positive controls: the SHIPPED CLI (--release, no feature) refuses the regtest receipt
run 1 "unknown network" $S --network regtest --expect 112:$A112 --liveness $T/xa.hex "$X@112$CXA" $RA
run 1 "receipt verifies against image" $S --expect 112:$A112 --liveness $T/xa.hex "$X@112$CXA" $RA
run 1 "receipt verifies against image" $S --network signet --expect 112:$A112 --liveness $T/xa.hex "$X@112$CXA" $RA
echo "zapp-liveness test-regtest.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
