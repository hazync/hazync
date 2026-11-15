#!/bin/bash
# hazync-headers-api against a fake bitcoind (fake-bitcoind-rpc) serving REAL mainnet headers 0..200 and real
# transactions, with the follower's `published` file moved by hand. Usage: test_api.sh
# Optional end to end: HEADERS_RECEIPTS=<0..100 rcpt>:<101..200 rcpt> HEADERS_IMAGE=<hex> VERIFY=<swiftsync-verify>
# also verifies the service's answers with the real verifier against real receipts of the MMR image.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); API=$HERE/../hazync-headers-api
FX=$HERE/../../../../experimental/libzync-bench/headers
W=$(mktemp -d); PIDS=()
cleanup () { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null; done; rm -rf "$W"; }
trap cleanup EXIT
port () { python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'; }
fails=0
t () { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (got '$3', want '$2')"; fails=$((fails + 1)); fi; }
hashat () { awk -v h="$1" '$1==h {print $2}' "$FX/h0_200.txt" | python3 -c "import sys,hashlib;b=bytes.fromhex(sys.stdin.read().strip());print(hashlib.sha256(hashlib.sha256(b).digest()).digest()[::-1].hex())"; }
get () { curl -s -o "$W/body" -w '%{http_code}' "$@"; }
js () { python3 -c "import json,sys;d=json.load(open('$W/body'));print($1)"; }

printf 'fakeuser:%s\n' "$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')" > "$W/cookie"
echo '{}' > "$W/control.json"
RP=$(port); FAKE_LOG="$W/calls.log" "$HERE/fake-bitcoind-rpc" "$RP" "$W/cookie" "$FX/h0_200.txt" "$W/control.json" \
  "$FX/tx/tx_170_0.json" "$FX/tx/tx_170_1.json" "$FX/tx/tx_100002.jsonl" & PIDS+=($!)
mkdir -p "$W/hd" "$W/state"
echo "150 $(hashat 150)" > "$W/hd/published"
start () {   # start <port> [env...]: the service, waiting until it answers /status
  env BIND=127.0.0.1 PORT="$1" HEADERS_DIR="$W/hd" STATE_DIR="$W/state" RPC_URL="http://127.0.0.1:$RP/" RPC_COOKIE="$W/cookie" \
      POLL=0.5 RATE=1000 BURST=1000 "${@:2}" python3 "$API" >> "$W/api-$1.log" 2>&1 & PIDS+=($!)
  for _ in $(seq 1 100); do curl -s "http://127.0.0.1:$1/status" >/dev/null && return 0; sleep 0.1; done; return 1
}
wait_tip () { for _ in $(seq 1 100); do [ "$(curl -s "$B/status" | python3 -c 'import json,sys;print(json.load(sys.stdin)["tip"])')" = "$1" ] && return 0; sleep 0.1; done; return 1; }
AP=$(port); B=http://127.0.0.1:$AP; start "$AP"
wait_tip 150; t "syncs to the PUBLISHED tip (150), not the node's newest (200)" 0 $?

t "path at tip 150: 200" 200 "$(get "$B/path?h=37")"
t "  tip 150, 151 leaves" "150 151" "$(js "d['tip'], d['leaves']" | tr -d '(),')"
echo "200 $(hashat 200)" > "$W/hd/published"; wait_tip 200; t "follows the published tip to 200" 0 $?
for h in 37 170; do
  get "$B/path?h=$h" >/dev/null
  t "path of $h at tip 200 == the independent reference (mmr_path_${h}_tip200.txt)" "$(cat "$FX/mmr_path_${h}_tip200.txt")" "$(js "'\n'.join(d['path'])")"
  t "  and names block $h's hash" "$(hashat $h)" "$(js "d['hash']")"
done
t "MMR root at 201 leaves == the pinned reference" f0de733aec1e32c865566ca4e1e23fa40f56c599c2bdf2399dec502606bfe5e3 "$(js "d['mmr_root']")"
get "$B/path?h=37&tip=100" >/dev/null; t "a path for an earlier tip (100): the reference root at 101 leaves" \
  "e6c523b11ea97d01240f42613f3809698b0ab757557c496032d8a3466b1a8b95 6" "$(js "d['mmr_root'], len(d['path'])" | tr -d "(),'")"
t "block above the tip: 404" 404 "$(get "$B/path?h=201")"
t "a tip above the published one: 400" 400 "$(get "$B/path?h=1&tip=201")"
t "not a height: 400" 400 "$(get "$B/path?h=1e3")"
t "a POST: 405" 405 "$(get -X POST "$B/path?h=1")"
t "an unknown route: 404" 404 "$(get "$B/admin")"
t "behind a proxy prefix (/headers-api/path)" 200 "$(get "$B/headers-api/path?h=5")"

# a transaction: what the verifier needs to refuse an inner node as a transaction (raw tx, raw coinbase, branches)
TX=f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16
t "/tx for block 170's second transaction: 200" 200 "$(get "$B/tx?txid=$TX")"
cp "$W/body" "$W/tx.json"
t "  == the node fixture (txid pos header raw branch coinbase coinbase_branch)" ok "$(python3 -c "
import json; a=json.load(open('$W/tx.json')); b=json.load(open('$FX/tx/tx_170_1.json'))
ks=['txid','pos','header','raw','branch','coinbase','coinbase_branch','hash','height']
print('ok' if all(a[k]==b[k] for k in ks) else [k for k in ks if a[k]!=b[k]])")"
t "  and the block's MMR path (as /path)" "$(cat "$FX/mmr_path_170_tip200.txt")" "$(python3 -c "import json;print('\n'.join(json.load(open('$W/tx.json'))['path']))")"
t "a transaction the node does not know: 404" 404 "$(get "$B/tx?txid=${TX%?}0")"
t "a transaction above the published tip (block 100,002): 404" 404 "$(get "$B/tx?txid=$(python3 -c "import json;print(json.loads(open('$FX/tx/tx_100002.jsonl').readline())['txid'])")")"
t "not a txid: 400" 400 "$(get "$B/tx?txid=xyz")"
# what /tx costs the node: getblock <hash> 1 (txids; the fake refuses verbosity 2), and a second /tx in the same block
# is answered from the per-block cache (no getblock, no raw header, no coinbase lookup)
CB170=$(python3 -c "import json;print(json.load(open('$FX/tx/tx_170_0.json'))['txid'])")
t "  /tx asked getblock of block 170 with verbosity 1, and never verbosity 2" "1 0" "$(grep -c "^getblock \[\"$(hashat 170)\", 1\]" "$W/calls.log") $(grep -c '^getblock \[".*", 2\]' "$W/calls.log")"
: > "$W/calls.log"
t "  a second /tx in block 170 (its coinbase): 200" 200 "$(get "$B/tx?txid=$CB170")"
t "    from the block cache: only getrawtransaction <txid> reached the node" "getrawtransaction" "$(cut -d' ' -f1 "$W/calls.log" | sort -u | tr -d '\n')"

# a restart reads the cache and fetches nothing
kill "${PIDS[-1]}"; wait "${PIDS[-1]}" 2>/dev/null
AP2=$(port); B=http://127.0.0.1:$AP2; start "$AP2"; wait_tip 200
t "restart: the cache holds the 201 hashes" 1 "$(grep -c 'cache: 201 hashes' "$W/api-$AP2.log")"
get "$B/path?h=37" >/dev/null; t "  and answers the same path" "$(cat "$FX/mmr_path_37_tip200.txt")" "$(js "'\n'.join(d['path'])")"

# the node reorgs block 199 away: the held hashes no longer end on the published hash -> nothing is served
echo "{\"lie\": {\"199\": \"$(printf '%064d' 7)\"}}" > "$W/control.json"; echo "200 $(printf '%064d' 9)" > "$W/hd/published"
for _ in $(seq 1 50); do [ "$(get "$B/path?h=5")" = 503 ] && break; sleep 0.1; done
t "a published tip the node disagrees with: 503, no path" 503 "$(get "$B/path?h=5")"
t "  /status says why" 1 "$(curl -s "$B/status" | grep -c 'is not the published')"
echo '{}' > "$W/control.json"; echo "200 $(hashat 200)" > "$W/hd/published"; wait_tip 200
t "  and it recovers when they agree again" 200 "$(get "$B/path?h=37")"
t "  with the same path" "$(cat "$FX/mmr_path_37_tip200.txt")" "$(js "'\n'.join(d['path'])")"
t "the node's cookie was used (a wrong one: the sync fails)" 1 "$(printf 'x:y\n' > "$W/cookie.bad"; RPC_COOKIE="$W/cookie.bad" python3 -c "
import importlib.util,importlib.machinery,os,sys
l=importlib.machinery.SourceFileLoader('api','$API'); s=importlib.util.spec_from_loader('api',l); m=importlib.util.module_from_spec(s); l.exec_module(m)
try: m.RPC().one('getblockhash',1); print(0)
except Exception: print(1)")"

# rate limit: a burst of 3 per client; clients told apart by X-Forwarded-For from a trusted proxy (127.0.0.1)
AP3=$(port); B3=http://127.0.0.1:$AP3; start "$AP3" RATE=0.01 BURST=3
codes=$(for i in 1 2 3 4 5; do get -H 'X-Forwarded-For: 203.0.113.7' "$B3/path?h=1"; echo; done | tr '\n' ' ')
t "rate limit: client A's 4th and 5th requests are refused" "200 200 429 429 " "$(echo "$codes" | cut -d' ' -f2-6)"
t "  client B is not" 200 "$(get -H 'X-Forwarded-For: 198.51.100.9' "$B3/path?h=1")"

t "  /tx has its own, tighter limit (TX_BURST=2): 3rd /tx refused, /path still served" "200 200 429 200" "$(AP5=$(port); start "$AP5" RATE=0.01 BURST=10 TX_RATE=0.01 TX_BURST=2 STATE_DIR="$W/state5"
  wait_tip_at () { for _ in $(seq 1 100); do [ "$(curl -s "http://127.0.0.1:$AP5/status" | python3 -c 'import json,sys;print(json.load(sys.stdin)["tip"])')" = 200 ] && return 0; sleep 0.1; done; }
  wait_tip_at; for u in tx tx tx; do get -H 'X-Forwarded-For: 203.0.113.8' "http://127.0.0.1:$AP5/tx?txid=$TX"; echo; done | tr '\n' ' '
  get -H 'X-Forwarded-For: 203.0.113.8' "http://127.0.0.1:$AP5/path?h=1")"

# a slow /tx (getblock of a big block) must not hold up the sync: the sync has its own RPC connection
AP4=$(port); B=http://127.0.0.1:$AP4; mkdir -p "$W/state4"; echo "150 $(hashat 150)" > "$W/hd/published"
start "$AP4" STATE_DIR="$W/state4"; wait_tip 150
echo '{"slow_getblock": 5}' > "$W/control.json"
curl -s -o /dev/null "$B/tx?txid=$TX" & SLOW=$!
sleep 0.5; t0=$(date +%s.%N); echo "200 $(hashat 200)" > "$W/hd/published"; wait_tip 200; t1=$(date +%s.%N)
t "a slow /tx (getblock 5 s) does not hold up the sync: at the new tip in < 2.5 s" yes "$(python3 -c "print('yes' if $t1 - $t0 < 2.5 else 'no: %.1f s' % ($t1 - $t0))")"
wait $SLOW; echo '{}' > "$W/control.json"

# a reorg DURING the fetch: the first getblockhash of 120 answers an old-branch block (a real-looking header: block
# 120's with another nonce, linked to 119), later ones the new chain. Taken without linkage, the stale hash would sit
# at 120 for good under a correct top (and every path over it would be wrong); linked, the fetch is redone.
read -r STALE STALEHDR <<< "$(python3 -c "
import hashlib; d=lambda b: hashlib.sha256(hashlib.sha256(b).digest()).digest()
h=bytearray(bytes.fromhex([l.split()[1] for l in open('$FX/h0_200.txt') if l.split()[0]=='120'][0])); h[79]^=1
print(d(bytes(h))[::-1].hex(), bytes(h).hex())")"
echo "{\"lie_once\": {\"120\": \"$STALE\"}, \"headers\": {\"$STALE\": \"$STALEHDR\"}}" > "$W/control.json"
AP6=$(port); B=http://127.0.0.1:$AP6; mkdir -p "$W/state6"; start "$AP6" STATE_DIR="$W/state6"; wait_tip 200
t "reorg during the fetch: 120 was asked again after the old-branch answer" yes "$([ "$(grep -c '^getblockhash \[120\]' "$W/calls.log")" -ge 2 ] && echo yes || echo no)"
t "  the break in the chain was seen and the range fetched again" 1 "$(grep -c 'moved during the fetch' "$W/api-$AP6.log")"
get "$B/path?h=120" >/dev/null; t "  the held hash at 120 is the real one" "$(hashat 120)" "$(js "d['hash']")"
get "$B/path?h=37" >/dev/null; t "  block 37's path (over 120) == the reference" "$(cat "$FX/mmr_path_37_tip200.txt")" "$(js "'\n'.join(d['path'])")"
echo '{}' > "$W/control.json"

# a stale cookie (bitcoind restarted, the LoadCredential copy is old): 401 for ever, so after AUTH_FAILS syncs in a
# row the service exits non-zero for systemd (Restart=on-failure) to restart it with a fresh copy
AP7=$(port); env BIND=127.0.0.1 PORT="$AP7" HEADERS_DIR="$W/hd" STATE_DIR="$W/state7" RPC_URL="http://127.0.0.1:$RP/" \
  RPC_COOKIE="$W/cookie.bad" POLL=0.2 AUTH_FAILS=2 timeout 20 python3 "$API" > "$W/api-$AP7.log" 2>&1
t "a refused cookie, AUTH_FAILS=2 syncs in a row: the service exits 3 (not 124, still running at 20 s)" 3 $?
t "  and says why" 1 "$(grep -c 'refused the cookie 2 times in a row' "$W/api-$AP7.log")"

if [ -n "${HEADERS_RECEIPTS:-}" ]; then
  # end to end: the service's answers, checked by the real verifier against real receipts of the MMR image
  IFS=: read -r R1 R2 <<< "$HEADERS_RECEIPTS"
  get "$B/path?h=37&tip=200" >/dev/null; js "'\n'.join(d['path'])" > "$W/p37"
  "$VERIFY" headers --image "$HEADERS_IMAGE" --include 37:"$(hashat 37)" --path "$W/p37" "$R1" "$R2" > "$W/v1" 2>&1
  t "e2e: block 37 by the service's path: DEMO (valid, below minimum work)" 2 $?
  "$VERIFY" headers --image "$HEADERS_IMAGE" --include 38:"$(hashat 37)" --path "$W/p37" "$R1" "$R2" > /dev/null 2>&1
  t "e2e: the same path at another height: FAIL" 1 $?
  python3 -c "import json;print('\n'.join(json.load(open('$W/tx.json'))['path']))" > "$W/p170"
  "$VERIFY" headers --image "$HEADERS_IMAGE" --include 170:"$(hashat 170)" --path "$W/p170" --tx "$TX:$W/tx.json" "$R1" "$R2" > "$W/v2" 2>&1
  t "e2e: the service's /tx answer for the asked txid: DEMO" 2 $?
  t "  and the verdict line names the asked txid" 1 "$(grep -c "INCLUDED: block 170 $(hashat 170) .*with transaction $TX in it" "$W/v2")"
  # a lying service: asked about $TX, it answers with its (valid) answer for block 170's coinbase
  B=http://127.0.0.1:$AP2; get "$B/tx?txid=$CB170" >/dev/null; cp "$W/body" "$W/cb.json"
  "$VERIFY" headers --image "$HEADERS_IMAGE" --include 170:"$(hashat 170)" --path "$W/p170" --tx "$TX:$W/cb.json" "$R1" "$R2" > "$W/v3" 2>&1
  t "e2e: the answer for ANOTHER transaction (the coinbase) under the asked txid: FAIL" 1 $?
  t "  refused as another transaction" 1 "$(grep -c 'not the transaction asked about' "$W/v3")"
fi
[ "$fails" -eq 0 ] && echo HEADERS_API_TESTS_OK || { echo "HEADERS_API_TESTS_FAILED ($fails)"; exit 1; }
