#!/bin/bash
# zapp-statement's OK verdict on REAL proofs: the zAPP regtest harness's GPU receipts (scripts/zapp-regtest/receipts,
# image eb2cd424: a 112, b 113) and their trie snapshots, a balance statement over the six ALICE scripts (P2PK both key
# forms, P2PKH, P2WPKH, P2SH-P2WPKH, P2TR), signed with ALICE's keys (chain/scenarios.json: regtest only, published on
# purpose), and Core's `scantxoutset` of each script (scripts/zapp-regtest/oracle) as the independent answer: every
# entry's coin count and balance, and the total, must be Core's.
#   test-regtest.sh <zapp-statement built with --features regtest-test (dev profile)> <the shipped zapp-statement (--release)> [<zapp-statement.wasm>]
# Writes zapps/statement/test-regtest.out. Every report of the feature build starts "TEST IMAGE (regtest-test)".
V=$(realpath "$1"); S=$(realpath "$2"); WASM=${3:+$(realpath "$3")}
cd "$(dirname "$0")/../.." || exit 1
exec > zapps/statement/test-regtest.out 2>&1
BAD=0; N=0
G=scripts/zapp-regtest; O="python3 $G/oracle_node.py"
T=$(mktemp -d); trap 'rm -rf $T' EXIT
A112=$($O a getblockhash 112); B112=$($O b getblockhash 112); B113=$($O b getblockhash 113)
# ALICE's keys as WIF (testnet/regtest prefix 0xef; compressed except the P2PK-uncompressed key) and her six scripts
python3 - $G/chain/scenarios.json $T/keys $T/scripts <<'P'
import hashlib, json, sys
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
def b58c(b):
    b += hashlib.sha256(hashlib.sha256(b).digest()).digest()[:4]
    n, s = int.from_bytes(b, "big"), ""
    while n:
        n, r = divmod(n, 58); s = B58[r] + s
    return "1" * (len(b) - len(b.lstrip(b"\0"))) + s
k = json.load(open(sys.argv[1]))["keys"]["ALICE"]
open(sys.argv[2], "w").write("".join(b58c(b"\xef" + bytes.fromhex(v["secret"]) + (b"" if t == "p2pk_u" else b"\x01")) + "\n" for t, v in k.items()))
open(sys.argv[3], "w").write(" ".join(v["scripts"][t]["spk"] for t, v in k.items()))
P
chmod 600 $T/keys
# oracle <snapshot> <statement.json>: every entry's "<coins>:<sat>" and the total equal Core's scantxoutset of its script
oracle() { python3 - "$G/oracle/snap_$1.json" "$2" <<'P'
import json, sys
o = {v["spk"]: v for k, v in json.load(open(sys.argv[1]))["scantxoutset"].items() if k.startswith("ALICE.")}
s = json.load(open(sys.argv[2]))
bad = [e["fact"]["script_pubkey"] for e in s["entries"]
       if e["fact"]["expect"] != f"{o[e['fact']['script_pubkey']]['n']}:{o[e['fact']['script_pubkey']]['total_amount_sat']}"]
tot = sum(v["total_amount_sat"] for v in o.values())
print(f"Core's scantxoutset: {sum(v['n'] for v in o.values())} coins, {tot} sat over {len(o)} scripts; the statement: total {s['total']}, "
      f"{len(s['entries'])} entries, {len(bad)} differing from Core")
sys.exit(0 if not bad and s["total"] == tot and len(s["entries"]) == len(o) == 6 else 1)
P
}
run() { local want=$1; local grepfor=$2; shift 2
        echo "== $* (expect exit $want)" | sed "s#$T#<tmp>#g; s#$V#zapp-statement[regtest-test]#; s#$S#zapp-statement[shipped]#"
        local out; out=$("$@" 2>&1); local got=$?
        echo "$out" | grep -a 'ZAPP_STATEMENT_\|\[FAIL\]\|TEST IMAGE\|\[ok\] statement:' | sed "s#$T#<tmp>#g"
        N=$((N+1))
        local marked=1 tl; tl=$(echo "$out" | grep -ac '^  \[ok\] TEST IMAGE (regtest-test): never a release' || true)
        if [ "$1" = "$V" ]; then [ "$tl" = 1 ] || marked=0; else [ "$tl" = 0 ] || marked=0; fi
        if [ "$got" = "$want" ] && [ "$marked" = 1 ] && { [ -z "$grepfor" ] || echo "$out" | grep -aq -- "$grepfor"; }; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
C="--purpose regtest-audit --nonce 7e57"
RA=$G/receipts/a/block_112.rcpt; RB=$G/receipts/b/block_113.rcpt
for x in a:112:a112 b:113:b113; do
  b=${x%%:*}; r=${x#*:}; h=${r%%:*}; lab=${r#*:}
  $V draft --network regtest --receipt $G/receipts/$b/block_$h.rcpt --trie $G/receipts/$b/state_$h.trie $C $(cat $T/scripts) > $T/s_$lab.json 2>$T/err \
    && $V sign $T/s_$lab.json --keys $T/keys 2>>$T/err || { cat $T/err; echo "draft/sign $lab: WRONG"; BAD=$((BAD+1)); }
  N=$((N+1)); if oracle $lab $T/s_$lab.json; then echo "statement $lab = Core's scantxoutset: PASS"; else echo "statement $lab != Core's scantxoutset: WRONG"; BAD=$((BAD+1)); fi
done
TA=$(python3 -c "import json; print(json.load(open('$T/s_a112.json'))['total'])"); TB=$(python3 -c "import json; print(json.load(open('$T/s_b113.json'))['total'])")
# ---- OK: each branch's statement on its own receipt, its tip expected / from a node on that branch
run 0 "$TA sat in total at block 112 $A112; each script signed it" $V --network regtest --expect 112:$A112 $C $RA $T/s_a112.json
run 0 "$TB sat in total at block 113 $B113; each script signed it" $V --network regtest --expect 113:$B113 $C $RB $T/s_b113.json
run 0 "ZAPP_STATEMENT_OK" $V --network regtest --bitcoin-cli "$O a -regtest" --max-age 0 $C $RA $T/s_a112.json
run 0 "ZAPP_STATEMENT_OK" $V --network regtest --bitcoin-cli "$O b -regtest" --max-age 0 $C $RB $T/s_b113.json
# ---- the reorg: a's statement against the active chain (b), b's tip expected, b's receipt: FAIL
run 1 "block 112 is $B112" $V --network regtest --bitcoin-cli "$O b -regtest" $C $RA $T/s_a112.json
run 1 "" $V --network regtest --expect 112:$B112 $C $RA $T/s_a112.json
run 1 "" $V --network regtest --expect 113:$B113 $C $RA $T/s_a112.json
run 1 "statement:" $V --network regtest --expect 113:$B113 $C $RB $T/s_a112.json
# ---- tampering and the challenge
python3 -c "import json,sys; s=json.load(open(sys.argv[1])); s['total']+=1; json.dump(s,open(sys.argv[2],'w'))" $T/s_a112.json $T/t.json
run 1 "statement:" $V --network regtest --expect 112:$A112 $C $RA $T/t.json
run 1 "" $V --network regtest --expect 112:$A112 --purpose other --nonce 7e57 $RA $T/s_a112.json
# no challenge, no identity: DEMO
run 2 "ZAPP_STATEMENT_DEMO" $V --network regtest $RA $T/s_a112.json
# ---- positive controls: the SHIPPED CLI (--release, no feature) refuses the regtest receipt, and cannot draft on it
run 1 "unknown network" $S --network regtest --expect 112:$A112 $C $RA $T/s_a112.json
run 1 "receipt verifies against image" $S --expect 112:$A112 $C $RA $T/s_a112.json
run 1 "receipt verifies against image" $S --network signet --expect 112:$A112 $C $RA $T/s_a112.json
run 1 "unknown network" $S draft --network regtest --receipt $RA --trie $G/receipts/a/state_112.trie $C $(cat $T/scripts)
# ---- and the shipped WASM module (its built .wasm, through its glue)
if [ -n "$WASM" ]; then
  N=$((N+1))
  if node --input-type=module - "$WASM" "$RA" "$T/s_a112.json" "$A112" <<'J'
import fs from 'node:fs';
import { loadStatementVerifier } from './zapps/statement/wasm/zapp-statement.js';
const [wasm, rc, st, a112] = process.argv.slice(2);
const v = await loadStatementVerifier(fs.readFileSync(wasm));
let bad = 0;
for (const network of ['mainnet', 'signet', 'regtest']) {
  const r = v.verifyStatement({ network, receipt: fs.readFileSync(rc), statement: JSON.parse(fs.readFileSync(st, 'utf8')),
    expect: `112:${a112}`, purpose: 'regtest-audit', nonce: '7e57' });
  console.log(`  wasm, network ${network}: ${r.verdict} (want fail)`);
  bad += r.verdict !== 'fail';
}
process.exit(bad ? 1 : 0);
J
  then echo "the shipped WASM module refuses the regtest receipt: PASS"; else echo "the shipped WASM module: WRONG"; BAD=$((BAD+1)); fi
fi
echo "zapp-statement test-regtest.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
