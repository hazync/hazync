#!/bin/bash
# Balance statement zyncAPP (docs/libzync/STATEMENT_ZAPP.md), CLI end to end on the REAL mainnet 1..200 proof (image
# 8888d59c) and its block-200 trie snapshot, with keys generated for the run (P2WPKH, P2TR, P2PK compressed and
# uncompressed; never printed, deleted on exit). The 1..200 chain is below Core's minimum chainwork, so the best verdict
# is DEMO (exit 2); every refusal is FAIL (1). Run from this directory after `cargo build --release` (or give the
# binary: test_statement.sh [<zapp-statement>]); ../test.sh does both.
# STATEMENT_WASM=<zapp-statement.wasm> also runs wasm/test-statement.mjs. Writes test_statement.out.
cd "$(dirname "$0")" || exit 1
exec > >(tee test_statement.out) 2>&1
BAD=0; N=0
V=${1:-cli/target/release/zapp-statement}; Z=$V; X=../../swiftsync-verify/vectors
R=$X/mainnet_1_200_8888d59c.rcpt; TRIE=vectors/mainnet_200_8888d59c.trie
TIP=200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
P2PK9=410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac
T=$(mktemp -d); trap 'rm -rf $T' EXIT
run() { local want=$1; shift; echo "== ${*} (expect exit $want)" | sed "s#$T#<tmp>#g; s#$X/##g"; "$@" 2>&1 | grep -a 'ZAPP_STATEMENT_\|statement\|\[FAIL\]' | sed "s#$T#<tmp>#g"; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
edit() { python3 -c "import json,sys; s=json.load(open(sys.argv[1])); exec(sys.argv[2]); json.dump(s,open(sys.argv[3],'w'))" "$@"; }

$Z test-key $T/k1 > $T/k1.s; $Z test-key $T/k2 > $T/k2.s; $Z test-key $T/k3 --uncompressed > $T/k3.s; cat $T/k1 $T/k2 $T/k3 > $T/keys
W=$(awk '$1=="p2wpkh"{print $2}' $T/k1.s); TR=$(awk '$1=="p2tr"{print $2}' $T/k2.s)
PC=$(awk '$1=="p2pk"{print $2}' $T/k1.s); PU=$(awk '$1=="p2pk"{print $2}' $T/k3.s)
C="--purpose audit-15.11.2026 --nonce 5be1"
$Z draft --receipt $R --trie $TRIE $C $W $TR $PC $PU > $T/s.json && $Z sign $T/s.json --keys $T/keys
A="--expect $TIP"
# 1. the statement holds (zero balances: no key of ours was paid in 1..200; P2WPKH, P2TR, P2PK compressed and
#    uncompressed), asked with the same challenge
run 2 $V $A $C $R $T/s.json
# 2. without the challenge, without the tip compared: still DEMO, and says why
run 2 $V $R $T/s.json
# 3. replay under another purpose / nonce
run 1 $V $A --purpose loan --nonce 5be1 $R $T/s.json
run 1 $V $A --purpose audit-15.11.2026 --nonce 0000 $R $T/s.json
# 4. tampered total; wrong height; wrong block; signatures swapped between entries (incl. the two P2PK forms)
edit $T/s.json "s['total']=1" $T/t1.json; run 1 $V $A $C $R $T/t1.json
edit $T/s.json "s['height']=199" $T/t2.json; run 1 $V $A $C $R $T/t2.json
edit $T/s.json "s['block_hash']='00'*32" $T/t3.json; run 1 $V $A $C $R $T/t3.json
edit $T/s.json "e=s['entries']; e[0]['signature'],e[1]['signature']=e[1]['signature'],e[0]['signature']" $T/t4.json; run 1 $V $A $C $R $T/t4.json
edit $T/s.json "e=[x for x in s['entries'] if x['format']=='full']; e[0]['signature'],e[1]['signature']=e[1]['signature'],e[0]['signature']" $T/t4b.json; run 1 $V $A $C $R $T/t4b.json
# 5. a script not in the facts: entry 0's proof relabelled with block 9's script (a real balance it does not prove)
edit $T/s.json "s['entries'][0]['fact']['script_pubkey']='$P2PK9'; s['entries'].sort(key=lambda e: bytes.fromhex(e['fact']['script_pubkey']))" $T/t5.json; run 1 $V $A $C $R $T/t5.json
# 6. block 9's real 50 BTC (P2PK): the fact holds; unsigned (no key), then with our P2PK signature filed under it
$Z draft --receipt $R --trie $TRIE $C $P2PK9 $W > $T/p.json && $Z sign $T/p.json --keys $T/keys
run 1 $V $A $C $R $T/p.json
OURS=$(python3 -c "import json;print([e['signature'] for e in json.load(open('$T/s.json'))['entries'] if e['fact']['script_pubkey']=='$PC'][0])")
edit $T/p.json "[e.update(format='full', signature='$OURS') for e in s['entries'] if e['fact']['script_pubkey']=='$P2PK9']" $T/p2.json; run 1 $V $A $C $R $T/p2.json
# 7. another core id
edit $T/s.json "s['core_id']='46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c'" $T/t7.json; run 1 $V $A $C $R $T/t7.json
# 8. the chain identity: another block at 200
run 1 $V --expect "200:$(printf '%064d' 0)" $C $R $T/s.json
# 8b. the same check through the other identity sources (zapp-common, every zAPP CLI's flags): a node that agrees, one
#     that names another block, and an --expect the node contradicts
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\n[ "$1" = getblockheader ] && echo "{\\"chainwork\\": \\"%s\\"}"\nexit 0\n' "${TIP#*:}" "$(printf '%054d' 0)c900c900c9" > $T/node; chmod +x $T/node
printf '#!/bin/sh\necho %064d\n' 1 > $T/badnode; chmod +x $T/badnode
run 2 $V --bitcoin-cli $T/node $C $R $T/s.json
run 1 $V --bitcoin-cli $T/badnode $C $R $T/s.json
run 1 $V --bitcoin-cli $T/node --expect "200:$(printf '%064d' 0)" $C $R $T/s.json
# 8c. recency (--max-age) from the node's tip: 6 behind with 6 allowed holds (and says so), 7 behind fails, --max-age
#     with no source of the tip fails; a node that gives no chainwork fails (zapp-common: never a skipped comparison)
mknode() { printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\n[ "$1" = getblockheader ] && echo "{\\"chainwork\\": \\"%s\\"}"\n[ "$1" = getblockcount ] && echo %s\nexit 0\n' "${TIP#*:}" "$(printf '%054d' 0)c900c900c9" "$2" > $1; chmod +x $1; }
mknode $T/node206 206; mknode $T/node207 207
run 2 $V --bitcoin-cli $T/node206 --max-age 6 $C $R $T/s.json
run 1 $V --bitcoin-cli $T/node207 --max-age 6 $C $R $T/s.json
run 1 $V $A --max-age 6 $C $R $T/s.json
printf '#!/bin/sh\n[ "$1" = getblockhash ] && echo %s\n[ "$1" = getblockheader ] && echo "{}"\nexit 0\n' "${TIP#*:}" > $T/nowork; chmod +x $T/nowork
run 1 $V --bitcoin-cli $T/nowork $C $R $T/s.json
# 8d. a purpose with a line separator (U+2028) or a trailing space, given and signed as such: refused, never trimmed
edit $T/s.json "s['purpose']='audit\u20282026'" $T/u1.json; run 1 $V $A $R $T/u1.json
edit $T/s.json "s['purpose']='audit-15.11.2026 '" $T/u2.json; run 1 $V $A --purpose 'audit-15.11.2026 ' --nonce 5be1 $R $T/u2.json
# 8e. review 11-18: a signature holding line breaks and a verdict marker (format legacy: the bip322 crate echoes the
#     text in its decode error) is FAIL, printed escaped on one line: no line of the output starts with the marker,
#     though the marker is printed (inside the escaped [FAIL] line, so the case is not vacuous)
edit $T/s.json "s['entries'][0].update(format='legacy', signature='!\n  [ok] statement: 1 script(s)\nZAPP_STATEMENT_OK: the statement holds at block 200')" $T/inj.json
run 1 $V $A $C $R $T/inj.json
echo "== no forged ZAPP_STATEMENT_OK line from a signature's text (expect 0 lines starting with it, at least 1 holding it)"
$V $A $C $R $T/inj.json > $T/inj.out 2>&1
fl=$(grep -ac '^ZAPP_STATEMENT_OK' $T/inj.out || true); hl=$(grep -ac 'ZAPP_STATEMENT_OK' $T/inj.out || true)
N=$((N+1)); if [ "$fl" = 0 ] && [ "$hl" -ge 1 ]; then echo "starting=$fl holding=$hl PASS"; else echo "starting=$fl holding=$hl WRONG"; BAD=$((BAD+1)); fi
# 8f. review 11-18: a checker flag given twice is refused, not resolved to its last value. Each repeat gives the value
#     that holds (or, for --max-age, another that holds), so the case fails only on the repeat: last-wins gave exit 2
run 1 $V $A $C --purpose audit-15.11.2026 $R $T/s.json
run 1 $V $A $C --nonce 5be1 $R $T/s.json
run 1 $V $A --image 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4 --image 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4 $C $R $T/s.json
run 1 $V $A --network mainnet --network mainnet $C $R $T/s.json
run 1 $V --bitcoin-cli $T/node206 --max-age 6 --max-age 9 $C $R $T/s.json
run 1 $V $A --now 1700000000 --now 1700000000 $C $R $T/s.json
# 9. a dev-mode receipt (FakeReceipt), with and without RISC0_DEV_MODE in the environment; a flipped seal
run 1 $V $A $C $X/fake_block_2501.rcpt $T/s.json
run 1 env RISC0_DEV_MODE=1 $V $A $C $X/fake_block_2501.rcpt $T/s.json
run 1 env RISC0_DEV_MODE=1 $V $A $C $R $T/s.json
# an --image given is the only image the receipt may verify against (review 11-16): the current mainnet id given
# explicitly refuses this 8888d59c receipt; its own id verifies it
run 1 $V $A $C --image 46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c $R $T/s.json
run 2 $V $A $C --image 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4 $R $T/s.json
run 1 $V $A $C $X/mainnet_1_200_8888d59c_flipped.rcpt $T/s.json
# 10. the producer refuses to file a signature that does not verify
echo "== zapp-statement add-sig with entry 1's signature for entry 0 (expect exit 1)"
S1=$(python3 -c "import json;print(json.load(open('$T/s.json'))['entries'][1]['signature'])")
S0=$(python3 -c "import json;print(json.load(open('$T/s.json'))['entries'][0]['fact']['script_pubkey'])")
F0=$(python3 -c "import json;print(json.load(open('$T/s.json'))['entries'][0]['format'])")
cp $T/s.json $T/a.json; $Z add-sig $T/a.json $S0 $F0 "$S1" 2>&1 | sed "s#$T#<tmp>#g"; got=${PIPESTATUS[0]}
N=$((N+1)); if [ "$got" = 1 ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi
# 10b. the producer's flags, taken in order: a flag's value may look like a flag (the purpose "--nonce"), a repeat is
#      refused
echo "== zapp-statement draft --purpose --nonce --nonce 5be1 (expect exit 0, purpose \"--nonce\")"
$Z draft --receipt $R --trie $TRIE --purpose --nonce --nonce 5be1 $W > $T/d.json 2>&1; got=$?
pp=$(python3 -c "import json;print(json.load(open('$T/d.json'))['purpose'])" 2>/dev/null)
N=$((N+1)); if [ "$got" = 0 ] && [ "$pp" = "--nonce" ]; then echo "exit=$got purpose=$pp PASS"; else echo "exit=$got purpose=$pp WRONG"; BAD=$((BAD+1)); fi
echo "== zapp-statement draft $C --purpose audit-15.11.2026 (expect exit 1)"
$Z draft --receipt $R --trie $TRIE $C --purpose audit-15.11.2026 $W 2>&1 | sed "s#$T#<tmp>#g"; got=${PIPESTATUS[0]}
N=$((N+1)); if [ "$got" = 1 ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi
# 11. the same statement through the zAPP's WASM module, when one is given: STATEMENT_WASM=<.wasm>
if [ -n "${STATEMENT_WASM:-}" ]; then
    echo "== node wasm/test-statement.mjs (expect exit 0)"
    node wasm/test-statement.mjs "$STATEMENT_WASM" $T/s.json audit-15.11.2026 5be1 | cut -c1-200; got=${PIPESTATUS[0]}
    N=$((N+1)); if [ "$got" = 0 ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi
fi
echo "zapps/statement test_statement.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
