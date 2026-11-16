#!/bin/bash
# zapp-deposit's OK verdict on REAL proofs: the zAPP regtest harness's GPU receipts (scripts/zapp-regtest/receipts,
# image eb2cd424: a 112, b 113) and each branch's history.bin, core proof only (the header-chain guest is mainnet only,
# so no burial beyond the core tip). Bundles are built from the blocks Core serves (oracle_node.py getblock), and every
# expectation checked is Core's (scripts/zapp-regtest/oracle): the block's hash and the transaction's position from
# Core's block, the output's value and script from Core's gettxout (include_mempool=false); the OP_RETURN payloads are
# the generator's (chain/scenarios.json), in Core-accepted blocks.
#   test-regtest.sh <zapp-deposit built with --features regtest-test (dev profile)> <the shipped zapp-deposit (--release)> [<zapp-deposit.wasm>]
# Writes zapps/deposit/test-regtest.out. Every report of the feature build starts "TEST IMAGE (regtest-test)".
V=$(realpath "$1"); S=$(realpath "$2"); WASM=${3:+$(realpath "$3")}
cd "$(dirname "$0")/../.." || exit 1
exec > zapps/deposit/test-regtest.out 2>&1
BAD=0; N=0
G=scripts/zapp-regtest; O="python3 $G/oracle_node.py"
T=$(mktemp -d); trap 'rm -rf $T' EXIT
A112=$($O a getblockhash 112); B112=$($O b getblockhash 112); B113=$($O b getblockhash 113)
sc() { python3 -c "import json,sys; v=json.load(open('$G/chain/scenarios.json'))['scenarios']; print(eval(sys.argv[1]))" "$1"; }
D=$(sc "v['reorg']['D']['txid']"); DOP=$(sc "v['reorg']['D']['op_return_payload']")
P=$(sc "v['deposit_102']['txid']"); POP=$(sc "v['deposit_102']['op_return_payload']")
T64=$(sc "v['tx_64_bytes_105']['txid']"); T105=$(sc "v['legacy_spends_105']['p2pk_c']")
T104=$(python3 -c "import json; print(json.load(open('$G/oracle/blocks_main.json'))['104']['txids'][-1])")
# core <snapshot> <txid> <vout>: "--amount <sat> --script <hex>" from Core's gettxout
core() { python3 -c "import json,sys; c=json.load(open('$G/oracle/snap_$1.json'))['gettxout']['$2:$3']; print('--amount', c['value_sat'], '--script', c['spk'])"; }
# where <branch> <txid>: "<height> <block hash> <position>" from Core's blocks
where() { python3 -c "
import json
for src in ('main', '$1'):
    for h, b in json.load(open('$G/oracle/blocks_' + src + '.json')).items():
        if '$2' in b['txids']: print(h, b['hash'], b['txids'].index('$2'))"; }
build() { local br=$1 h=$2 tx=$3 out=$4; shift 4
          $V build --network regtest --core $G/receipts/$br/block_$h.rcpt --history $G/receipts/$br/history.bin --txid $tx "$@" -o $T/$out.json; }
read DH DHASH DI <<< "$(where a $D)"; read PH PHASH PI <<< "$(where a $P)"; read H104 X104 I104 <<< "$(where a $T104)"; read H64 X64 I64 <<< "$(where a $T64)"; read H105 X105 I105 <<< "$(where a $T105)"
echo "Core: D in a $DH $DHASH at $DI; P in $PH $PHASH at $PI; block 104's last tx at $I104; the 64-byte tx in $H64 at $I64; the P2PK spend of 105 at $I105"
N=$((N+1))
if build a 112 $D d_a --height $DH --bitcoin-cli "$O a -regtest" && build a 112 $P p_a --height $PH --bitcoin-cli "$O a -regtest" \
   && build b 113 $P p_b --height $PH --bitcoin-cli "$O b -regtest" && build a 112 $T104 t104 --height $H104 --bitcoin-cli "$O a -regtest" \
   && build a 112 $T105 t105 --height $H105 --bitcoin-cli "$O a -regtest"; then echo "bundles built: PASS"; else echo "bundles built: WRONG"; BAD=$((BAD+1)); fi
run() { local want=$1; local grepfor=$2; shift 2
        echo "== $* (expect exit $want)" | sed "s#$T#<tmp>#g; s#$V#zapp-deposit[regtest-test]#; s#$S#zapp-deposit[shipped]#"
        local out; out=$("$@" 2>&1); local got=$?
        echo "$out" | grep -a 'VERIFY_\|\[FAIL\]\|TEST IMAGE\|^zapp-deposit' | sed "s#$T#<tmp>#g"
        N=$((N+1))
        # the feature build's verifier reports start with the TEST IMAGE line; the shipped one never has it
        local marked=1 first; first=$(echo "$out" | grep -a '^  \[' | head -1)
        if [ "$1" = "$V" ]; then case "$first" in *"] core: TEST IMAGE (regtest-test): never a release") ;; *) [ "$2" = build ] || marked=0;; esac
        else echo "$out" | grep -aq "TEST IMAGE" && marked=0; fi
        if [ "$got" = "$want" ] && [ "$marked" = 1 ] && { [ -z "$grepfor" ] || echo "$out" | grep -aq -- "$grepfor"; }; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
R="$V --network regtest"
# ---- OK: D in a 111 under 2 blocks; its output and its OP_RETURN; the 102 deposit on both branches; block 104's last
# of 17 transactions (odd count); a transaction of block 105, beside its real 64-byte one
run 0 "ACCEPTED: deposit: transaction $D, output 0 = .* is in block $DH $DHASH, 2 confirmation(s) as of tip 112 $A112" \
    $R --txid $D --vout 0 $(core a112 $D 0) --min-depth 2 --expect 112:$A112 $T/d_a.json
run 0 "ACCEPTED" $R --txid $D --vout 1 --op-return $DOP --min-depth 2 --expect 112:$A112 $T/d_a.json
run 0 "ACCEPTED" $R --txid $D --op-return $DOP --min-depth 2 --bitcoin-cli "$O a -regtest" $T/d_a.json
run 0 "is in block $PH $PHASH, 11 confirmation(s) as of tip 112 $A112" $R --txid $P --vout 0 $(core a112 $P 0) --min-depth 11 --expect 112:$A112 $T/p_a.json
run 0 "is in block $PH $PHASH, 12 confirmation(s) as of tip 113 $B113" $R --txid $P --vout 1 --op-return $POP --min-depth 12 --expect 113:$B113 $T/p_b.json
run 0 "is in block $PH $PHASH, 12 confirmation(s) as of tip 113 $B113" $R --txid $P --vout 0 $(core b113 $P 0) --min-depth 12 --bitcoin-cli "$O b -regtest" $T/p_b.json
run 0 "ACCEPTED: deposit: transaction $T104 is in block $H104 $X104" $R --txid $T104 --min-depth 9 --expect 112:$A112 $T/t104.json
run 0 "ACCEPTED: deposit: transaction $T105 is in block $H105 $X105" $R --txid $T105 --min-depth 8 --expect 112:$A112 $T/t105.json
# the real 64-byte transaction of block 105 (Core accepted it): refused BY DESIGN (DEPOSIT_ZAPP.md: a transaction whose
# txid preimage is 64 bytes cannot be told apart from an inner merkle node; BIP 54), so no bundle is written
run 1 "not exactly 64 bytes" $V build --network regtest --core $G/receipts/a/block_112.rcpt --history $G/receipts/a/history.bin --txid $T64 --height $H64 --bitcoin-cli "$O a -regtest" -o $T/t64.json
# ---- FAIL: deeper than shown, another amount, another transaction asked
run 1 "" $R --txid $D --vout 0 --min-depth 3 --expect 112:$A112 $T/d_a.json
run 1 "" $R --txid $D --vout 0 --amount 250000001 --min-depth 2 --expect 112:$A112 $T/d_a.json
run 1 "" $R --txid $P --min-depth 1 --expect 112:$A112 $T/d_a.json
# ---- the reorg: D is absent on b. Against b's tip (expected, or the node on the active chain) a's bundle FAILs, and no
# bundle can be built on b's proof: a's block 111 is not b's, and b's block 111 does not hold D
run 1 "" $R --txid $D --vout 0 --min-depth 2 --expect 113:$B113 $T/d_a.json
run 1 "" $R --txid $D --vout 0 --min-depth 2 --expect 112:$B112 $T/d_a.json
run 1 "" $R --txid $D --vout 0 --min-depth 2 --bitcoin-cli "$O b -regtest" $T/d_a.json
run 1 "not the proven chain's at that height" $V build --network regtest --core $G/receipts/b/block_113.rcpt --history $G/receipts/b/history.bin --txid $D --height $DH --bitcoin-cli "$O a -regtest" -o $T/x.json
run 1 "the transaction is not in that block" $V build --network regtest --core $G/receipts/b/block_113.rcpt --history $G/receipts/b/history.bin --txid $D --height $DH --bitcoin-cli "$O b -regtest" -o $T/x.json
# no tip source: DEMO, not accepted
run 2 "deposit NOT accepted" $R --txid $D --vout 0 --min-depth 2 $T/d_a.json
# ---- positive controls: the SHIPPED CLI (--release, no feature) refuses: no regtest network, the bundle's network is
# not mainnet, and a bundle relabelled mainnet does not verify against the mainnet image; nor can it build one
python3 -c "import json,sys; b=json.load(open(sys.argv[1])); b['network']='mainnet'; json.dump(b,open(sys.argv[2],'w'))" $T/d_a.json $T/d_m.json
run 1 "unknown network" $S --network regtest --txid $D --vout 0 --min-depth 2 --expect 112:$A112 $T/d_a.json
run 1 "the bundle is for" $S --txid $D --vout 0 --min-depth 2 --expect 112:$A112 $T/d_a.json
run 1 "receipt verifies against image" $S --txid $D --vout 0 --min-depth 2 --expect 112:$A112 $T/d_m.json
run 1 "unknown network" $S build --network regtest --core $G/receipts/a/block_112.rcpt --history $G/receipts/a/history.bin --txid $D --height $DH --bitcoin-cli "$O a -regtest" -o $T/x.json
# ---- and the shipped WASM module (its built .wasm, through its glue): both bundles refused
if [ -n "$WASM" ]; then
  N=$((N+1))
  if node --input-type=module - "$WASM" "$T/d_a.json" "$T/d_m.json" "$D" "$A112" <<'J'
import fs from 'node:fs';
import { loadDeposit } from './zapps/deposit/wasm/zapp-deposit.js';
const [wasm, da, dm, d, a112] = process.argv.slice(2);
const v = await loadDeposit(fs.readFileSync(wasm));
let bad = 0;
for (const [name, f, network] of [['regtest bundle', da, 'regtest'], ['regtest bundle as mainnet', da, 'mainnet'], ['relabelled mainnet', dm, 'mainnet']]) {
  const r = v.verifyDeposit({ bundle: fs.readFileSync(f, 'utf8'), ask: { txid: d, vout: 0, min_depth: 2, expect: `112:${a112}`, network } });
  console.log(`  wasm, ${name}: ${r.verdict} accepted=${r.accepted} (want fail)`);
  bad += r.verdict !== 'fail' || r.accepted;
}
process.exit(bad ? 1 : 0);
J
  then echo "the shipped WASM module refuses the regtest bundles: PASS"; else echo "the shipped WASM module: WRONG"; BAD=$((BAD+1)); fi
fi
echo "zapp-deposit test-regtest.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
