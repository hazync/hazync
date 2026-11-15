#!/bin/bash
# Statement of balance (docs/libzync/STATEMENT_ZAPP.md), CLI end to end on the REAL mainnet 1..200 proof (image 8888d59c)
# and its block-200 trie snapshot, with two keys generated for the run (P2WPKH and P2TR; never printed, deleted on exit).
# The 1..200 chain is below Core's minimum chainwork, so the best verdict is DEMO (exit 2); every refusal is FAIL (1).
# Run from the repo root after `cargo build --release` in swiftsync-verify. Writes swiftsync-verify/test_statement.out.
cd "$(dirname "$0")/.." || exit 1
exec > >(tee swiftsync-verify/test_statement.out) 2>&1
BAD=0; N=0
B=swiftsync-verify/target/release; V=$B/swiftsync-verify; Z=$B/zync-statement; X=swiftsync-verify/vectors
R=$X/mainnet_1_200_8888d59c.rcpt; TRIE=$X/mainnet_200_8888d59c.trie
TIP=200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320
P2PK9=410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac
T=$(mktemp -d); trap 'rm -rf $T' EXIT
run() { local want=$1; shift; echo "== ${*#$V } (expect exit $want)"; "$@" 2>&1 | grep -a 'VERIFY_\|statement\|\[FAIL\]' | sed "s#$T#<tmp>#g"; local got=${PIPESTATUS[0]}; N=$((N+1)); if [ "$got" = "$want" ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi; }
edit() { python3 -c "import json,sys; s=json.load(open(sys.argv[1])); exec(sys.argv[2]); json.dump(s,open(sys.argv[3],'w'))" "$@"; }

$Z test-key $T/k1 > $T/k1.scripts; $Z test-key $T/k2 > $T/k2.scripts; cat $T/k1 $T/k2 > $T/keys
W=$(awk '$1=="p2wpkh"{print $2}' $T/k1.scripts); TR=$(awk '$1=="p2tr"{print $2}' $T/k2.scripts)
C="--purpose audit-15.11.2026 --nonce 5be1"
$Z draft --receipt $R --trie $TRIE $C $W $TR > $T/s.json && $Z sign $T/s.json --keys $T/keys
A="--expect $TIP"
# 1. the statement holds (zero balances: no key of ours was paid in 1..200), asked with the same challenge
run 2 $V $A $C --statement $T/s.json $R
# 2. without the challenge: still DEMO, and says why
run 2 $V $A --statement $T/s.json $R
# 3. replay under another purpose / nonce
run 1 $V $A --purpose loan --nonce 5be1 --statement $T/s.json $R
run 1 $V $A --purpose audit-15.11.2026 --nonce 0000 --statement $T/s.json $R
# 4. tampered total; wrong height; wrong block; a signature swapped between the entries
edit $T/s.json "s['total']=1" $T/t1.json; run 1 $V $A $C --statement $T/t1.json $R
edit $T/s.json "s['height']=199" $T/t2.json; run 1 $V $A $C --statement $T/t2.json $R
edit $T/s.json "s['block_hash']='00'*32" $T/t3.json; run 1 $V $A $C --statement $T/t3.json $R
edit $T/s.json "e=s['entries']; e[0]['signature'],e[1]['signature']=e[1]['signature'],e[0]['signature']" $T/t4.json; run 1 $V $A $C --statement $T/t4.json $R
# 5. a script not in the facts: entry 0's proof relabelled with block 9's script (a real balance it does not prove)
edit $T/s.json "s['entries'][0]['fact']['script_pubkey']='$P2PK9'; s['entries'].sort(key=lambda e: bytes.fromhex(e['fact']['script_pubkey']))" $T/t5.json; run 1 $V $A $C --statement $T/t5.json $R
# 6. block 9's real 50 BTC (P2PK): the fact holds, but no BIP-322 signature can be checked for P2PK
$Z draft --receipt $R --trie $TRIE $C $P2PK9 $W > $T/p.json && $Z sign $T/p.json --keys $T/keys
run 1 $V $A $C --statement $T/p.json $R
# 7. a statement made under 8888d59c, checked as if by the current image's chain (another core id)
edit $T/s.json "s['core_id']='46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c'" $T/t7.json; run 1 $V $A $C --statement $T/t7.json $R
# 8. the chain identity: another block at 200
run 1 $V --expect 200:$(printf '%064d' 0) $C --statement $T/s.json $R
# 9. a dev-mode receipt (FakeReceipt), with and without RISC0_DEV_MODE in the environment; a flipped seal
run 1 $V $A $C --statement $T/s.json $X/fake_block_2501.rcpt
run 1 env RISC0_DEV_MODE=1 $V $A $C --statement $T/s.json $X/fake_block_2501.rcpt
run 1 $V $A $C --statement $T/s.json $X/mainnet_1_200_8888d59c_flipped.rcpt
# 10. the producer refuses to file a signature that does not verify
echo "== zync-statement add-sig with entry 1's signature for entry 0 (expect exit 1)"
S1=$(python3 -c "import json;print(json.load(open('$T/s.json'))['entries'][1]['signature'])")
S0SPK=$(python3 -c "import json;print(json.load(open('$T/s.json'))['entries'][0]['fact']['script_pubkey'])")
cp $T/s.json $T/a.json; $Z add-sig $T/a.json $S0SPK simple "$S1" 2>&1 | sed "s#$T#<tmp>#g"; got=${PIPESTATUS[0]}
N=$((N+1)); if [ "$got" = 1 ]; then echo "exit=$got PASS"; else echo "exit=$got WRONG"; BAD=$((BAD+1)); fi
echo "swiftsync-verify test_statement.sh: $((N-BAD))/$N as expected"
[ "$N" -gt 0 ] && [ "$BAD" = 0 ]
