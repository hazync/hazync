#!/bin/bash
# board_rehearsal.sh <host O> <host N> [--control]: the live board's handover (docs/HANDOVER.md §5), end to end on a
# local coordinator and the stock worker, mainnet blocks 1..END in DEV MODE (rules execute, nothing is proved).
#   O = an image with an empty lineage; N = the same source with O pinned at H (scratchpad build_on.sh builds both).
#   1. prep 1..END with N's binary: states below H must be byte-identical to O's own prep, from H on stamped N/L1.
#   2. the coordinator runs O's host with LZ_HANDOVER_AT=H ("pre"): an O worker proves 1..H-1 by claims, nothing at H
#      or above is handed out, and O's spine reaches exactly H-1.
#   3. the coordinator restarts on N's host ("post"), same database: O's ranges still verify, an O proof above H and
#      an N leaf at H (not the handover step) are refused.
#   4. an N worker proves H+1..END by claims (the first is H+1).
#   5. `hazync spine` with N proves block H as the handover and absorbs H+1..END: spine [1..END], frontier END,
#      vrange [1..H] recorded with image N.
#   6. verify-any (N) accepts the spine with lineage L1 = digest([O at H]) (block_forest_ref.py), and refuses the same
#      receipt with its lineage spliced to zero.
# --control: prep by O's binary and N with stamping switched off (HAZYNC_TEST_NO_STAMP): the 11-11 failure, where the
# new image's leaves never chain. The run MUST fail.
# Dev mode has no real seals, so the CROSS-VERSION check is not exercised here: experimental/libzync-bench/xver/ and
# `host xver-selftest` cover it on real receipts.
# Exit 0 only if every check came out as expected.
set -u
OH=$1; NH=$2; CONTROL=0; [ "${3:-}" = --control ] && CONTROL=1
H=${H:-11}; END=${END:-20}
J=${J:-$HOME/libzync-gpu/ssweep}       # mainnet block_<h>.json files (fetch_block.py)
HERE=$(cd "$(dirname "$0")" && pwd); C=$(cd "$HERE/../../../coordinator" && pwd); REF=$HERE/../../../scripts/libzync/block_forest_ref.py
D=${D:-$HERE/rehearsal_out}; rm -rf "$D"; mkdir -p "$D/bridge" "$D/wO" "$D/wN"
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2 HAZYNC_TEST_FAKE_RECEIPTS=1
NOSTAMP=(); [ $CONTROL = 1 ] && NOSTAMP=(HAZYNC_TEST_NO_STAMP=1)
BAD=0; N_CASES=0
pass() { N_CASES=$((N_CASES+1)); echo "PASS $1"; }
wrong() { N_CASES=$((N_CASES+1)); BAD=$((BAD+1)); echo "WRONG $1"; }
OID=$("$OH" method-id | sed -n 's/^METHOD_ID //p'); NID=$("$NH" method-id | sed -n 's/^METHOD_ID //p')
L1=$("$NH" libzync-lineage | sed -n 's/^LINEAGE //p')
"$NH" libzync-lineage | grep -q "first_height=$H " || { echo "N does not pin a handover at $H"; exit 1; }
echo "O $OID  N $NID  H $H  L1 $L1"

# 1. prep
PREP=$NH; [ $CONTROL = 1 ] && PREP=$OH
HAZYNC_PREP_LEAF_BUNDLES=$D/bridge "$PREP" libzync-prep 1 $END "$J" genesis "$D/prep" > "$D/prep.log" 2>&1 \
    || { tail -3 "$D/prep.log"; echo "prep failed"; exit 1; }
"$OH" libzync-prep 1 $END "$J" genesis "$D/prepO" > "$D/prepO.log" 2>&1 || { echo "O prep failed"; exit 1; }
python3 - "$D" "$H" "$END" "$OID" "$NID" "$L1" "$REF" "$CONTROL" <<'P' && pass "prep: states below $H are O's own, from $H stamped N with L1 (= the reference)" || wrong "prep stamping by era"
import importlib.util, json, struct, sys
D, H, END, oid, nid, l1, ref, control = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), *sys.argv[4:8]
s = importlib.util.spec_from_file_location("ref", ref); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
ow = list(struct.unpack("<8I", bytes.fromhex(oid))); nw = list(struct.unpack("<8I", bytes.fromhex(nid)))
assert m.lineage([(ow, H)]).hex() == l1, "N's lineage is not the reference's digest of [O at H]"
ok = True
for h in range(1, END + 1):
    a = json.load(open(f"{D}/prep/state_{h}.json")); b = json.load(open(f"{D}/prepO/state_{h}.json"))
    if h < H:
        ok &= a == b
    else:
        ok &= a["self_id"] == nw and bytes(a["lineage"]).hex() == l1
        b2 = dict(b); b2["self_id"] = nw; b2["lineage"] = a["lineage"]
        ok &= a == b2                      # nothing but the stamp differs
sys.exit(0 if ok or control == "1" else 1)
P
GT=$("$NH" libzync-genesis-tip); [ "$GT" = "$("$OH" libzync-genesis-tip)" ] && pass "GENESIS_TIP is the same under O and N" || wrong "GENESIS_TIP differs"

PORT=${PORT:-18941}; U=http://127.0.0.1:$PORT
start() {  # start <host> <log>
  ( cd "$C" && env "${NOSTAMP[@]}" COORD_ALLOW_MOCK=1 COORD_PORT=$PORT COORD_BIND=127.0.0.1 COORD_DB=$D/c.db COORD_SPINE=$D/spine \
    COORD_PROOFS=$D/proofs COORD_WEB=$C HAZYNC_HOST=$1 HAZYNC_LIBZYNC=1 HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep \
    GENESIS_TIP=$GT LZ_HANDOVER_AT=$H CLAIM_RETAKE_WAIT=0 exec python3 server.py > "$2" 2>&1 ) &
  SP=$!
  for i in $(seq 1 60); do curl -s $U/api/meta > /dev/null && break; sleep 1; done
}
WO="env HAZYNC_HOME=$D/wO COORD_URL=$U HAZYNC_HOST=$OH python3 $C/hazync"
WN="env ${NOSTAMP[*]} HAZYNC_HOME=$D/wN COORD_URL=$U HAZYNC_HOST=$NH python3 $C/hazync"
era() { curl -s $U/api/meta | python3 -c "import json,sys; h=json.load(sys.stdin).get('handover') or {}; print(h.get('era'), h.get('done'))"; }

# 2. pre era
start "$OH" "$D/server_pre.log"
[ "$(era)" = "pre False" ] && pass "coordinator on O's host is in the pre era" || wrong "pre era: $(era)"
$WO id > /dev/null 2>&1
got=""
for i in $(seq 1 $((H-1))); do
  $WO run > "$D/runO_$i.log" 2>&1; got="$got $(sed -n 's/^claimed block \([0-9]*\).*/\1/p' "$D/runO_$i.log")"
done
[ "$(echo $got)" = "$(seq -s ' ' 1 $((H-1)))" ] && pass "O's worker claimed and proved 1..$((H-1)) in order" || wrong "O's claims: $got"
$WO run > "$D/runO_more.log" 2>&1
grep -q "nothing to claim" "$D/runO_more.log" && pass "nothing at or above $H is handed out to O" || wrong "O was handed: $(grep -a claimed "$D/runO_more.log")"
$WO spine > "$D/spineO.log" 2>&1
[ "$(curl -s $U/api/spine | python3 -c 'import json,sys; print(json.load(sys.stdin).get("hi"))')" = $((H-1)) ] \
    && pass "O's spine is exactly [1..$((H-1))]" || wrong "O's spine: $(tail -2 "$D/spineO.log")"
kill $SP; wait $SP 2>/dev/null

# 3. post era: same database, N's host
start "$NH" "$D/server_post.log"
[ "$(era)" = "post False" ] && pass "coordinator on N's host is in the post era, handover not yet done" || wrong "post era: $(era)"
OL=$(env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep "$NH" verify-any "$D/wO/receipts/5.bin" 5 --check-end 2>&1 | grep -a '^RANGE-OK')
echo "$OL" | grep -q "image=$OID " && pass "N's verify-any accepts O's block 5 as O's (image=$OID)" || wrong "N on O's block 5: $OL"
# an O proof above H, and N's block H as a plain leaf instead of the handover step
( cd "$D" && HAZYNC_BRIDGE_OUT=$D/bridge "$OH" prove-range-bridge $((H+1)) > "$D/o_above.log" 2>&1 )
r=$(env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep "$NH" verify-any "$D/range_$((H+1)).bin" $((H+1)) 2>&1)
echo "$r" | grep -q "REJECT.*MISMATCH" && pass "an O proof of block $((H+1)) is refused as MISMATCH" || wrong "O proof above H: $(echo "$r" | tail -1)"
( cd "$D" && env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge "$NH" prove-range-bridge $H > "$D/n_leafH.log" 2>&1 )
r=$(env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep "$NH" verify-any "$D/range_$H.bin" $H 2>&1)
echo "$r" | grep -q "REJECT.*forged history" && pass "N's block $H as a plain leaf (not the handover) is refused: lineage" || wrong "N leaf at H: $(echo "$r" | tail -1)"

# 4. N proves above H
$WN id > /dev/null 2>&1
got=""
for i in $(seq $((H+1)) $END); do
  $WN run > "$D/runN_$i.log" 2>&1; got="$got $(sed -n 's/^claimed block \([0-9]*\).*/\1/p' "$D/runN_$i.log")"
done
[ "$(echo $got)" = "$(seq -s ' ' $((H+1)) $END)" ] && pass "N's worker claimed $((H+1))..$END in order (never $H)" || wrong "N's claims: $got"
nv=$(sqlite3 "$D/c.db" "SELECT COUNT(*) FROM vranges WHERE lo > $H")
[ "$nv" = $((END-H)) ] && pass "every N block above $H verified on the board ($nv)" || wrong "N blocks verified above $H: $nv of $((END-H)) ($(grep -ah 'rejected\|✗' "$D"/runN_*.log | head -1 | cut -c1-200))"

# 5. the spine crosses the handover
$WN spine > "$D/spineN.log" 2>&1
grep -q "handover: proving block $H" "$D/spineN.log" && pass "hazync spine proved block $H as the handover step" || wrong "no handover step: $(tail -2 "$D/spineN.log")"
SH=$(curl -s $U/api/spine | python3 -c 'import json,sys; print(json.load(sys.stdin).get("hi"))')
[ "$SH" = $END ] && pass "the spine is [1..$END]" || wrong "spine at $SH"
FR=$(curl -s $U/api/state | python3 -c 'import json,sys; s=json.load(sys.stdin); print(s.get("frontier", s.get("frontier_hi")))')
[ "$FR" = $END ] && pass "the frontier is $END" || wrong "frontier $FR"
VI=$(sqlite3 "$D/c.db" "SELECT image FROM vranges WHERE id='1-$H'")
[ "$VI" = "$NID" ] && pass "vrange [1..$H] recorded, image N" || wrong "vrange [1..$H]: '$VI'"
[ "$(era)" = "post True" ] && pass "/api/meta says the handover is done" || wrong "after: $(era)"

# 6. verifiers on the new spine
curl -s $U/api/spine/proof > "$D/spine.bin"
r=$(env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep "$NH" verify-any "$D/spine.bin" 1 --check-end 2>&1 | grep -a '^RANGE-OK')
echo "$r" | grep -q "hi=$END .*image=$NID lineage=$L1 anchored=1" && pass "verify-any: the spine is N's, lineage L1, genesis-anchored" || wrong "spine verify: $r"
python3 - "$D/spine.bin" "$D/spliced.bin" "$L1" <<'P'
import struct, sys
b = open(sys.argv[1], "rb").read()
old = b"".join(struct.pack("<I", x) for x in bytes.fromhex(sys.argv[3]))
n = b.count(old)
open(sys.argv[2], "wb").write(b.replace(old, b"".join(struct.pack("<I", 0) for _ in range(32))))
print(f"lineage occurrences spliced to zero: {n}")
P
r=$(env "${NOSTAMP[@]}" HAZYNC_BRIDGE_OUT=$D/bridge HAZYNC_LZ_STATE_DIR=$D/prep "$NH" verify-any "$D/spliced.bin" 1 2>&1)
echo "$r" | grep -q "^REJECT" && pass "the same spine with its lineage spliced to zero is refused ($(echo "$r" | grep -a REJECT | cut -c1-90))" || wrong "spliced lineage accepted: $r"
kill $SP; wait $SP 2>/dev/null
grep -a -i 'traceback' "$D"/server_*.log | head -3
echo "board_rehearsal.sh$([ $CONTROL = 1 ] && echo ' --control'): $((N_CASES-BAD))/$N_CASES as expected"
[ "$BAD" = 0 ]
