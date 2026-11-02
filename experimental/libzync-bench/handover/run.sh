#!/bin/bash
# run.sh <host O> <host N> <blocks dir> <out dir>: the handover and block forest on a real regtest chain, in dev mode
# (rules execute, nothing is proved). O has an empty lineage; N is the same source with O pinned at FIRST.
#   1. O proves 1..LAST from genesis. N refuses to be handed anything else, then continues to END by a handover.
#   2. The state across the handover is checked, and the forest against scripts/libzync/block_forest_ref.py.
#   3. Every guest check is reached alone, and must reject with its OWN message: a test that only sees "failed"
#      would pass on any failure. A control run with the same hook unset must succeed.
# Exit 0 only if every case came out as expected.
O=$1; N=$2; B=$3; R=$4; FIRST=${FIRST:-101}; LAST=$((FIRST-1)); END=${END:-$((FIRST+9))}
HERE=$(cd "$(dirname "$0")" && pwd); REF=$HERE/../../../scripts/libzync/block_forest_ref.py
export RISC0_DEV_MODE=1 HAZYNC_RIG=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_COINLOG_SAVE_EACH=1
BAD=0; N_CASES=0
mkdir -p "$R"
pass() { N_CASES=$((N_CASES+1)); echo "PASS $1"; }
wrong() { N_CASES=$((N_CASES+1)); BAD=$((BAD+1)); echo "WRONG $1"; }

# Block hashes for the reference, from each block's header fields. Each must equal the next block's `prev`, so a
# mistake here cannot pass quietly. Height 0 is the first block's `prev`.
python3 - "$B" "$END" "$R/hashes.txt" <<'P' || { echo "block hashes failed"; exit 1; }
import hashlib, json, struct, sys
B, end, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
J = {h: json.load(open(f"{B}/block_{h}.json")) for h in range(1, end + 2)}
hs = [J[1]["prev"]]
for h in range(1, end + 1):
    j = J[h]
    hdr = struct.pack("<i", j["version"]) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1] + struct.pack("<III", j["time"], j["bits"], j["nonce"])
    d = hashlib.sha256(hashlib.sha256(hdr).digest()).digest()[::-1].hex()
    assert d == J[h + 1]["prev"], (h, d, J[h + 1]["prev"])
    hs.append(d)
open(out, "w").write("\n".join(hs) + "\n")
P

"$O" libzync-prove-range 1 "$LAST" "$B" genesis "$R/o" > "$R/o.log" 2>&1 || { tail -5 "$R/o.log"; echo "O chain failed"; exit 1; }
pass "O proves 1..$LAST from genesis"

# case <name> <expected guest message> [receipt height]
neg() {
    local name=$1 want=$2 h=${3:-$LAST} from=$((${3:-$LAST}+1))
    rm -rf "$R/n_$name"
    HAZYNC_TEST_HANDOVER=$name HAZYNC_HANDOVER_RECEIPT=$R/o/block_$h.rcpt timeout 900 "$N" libzync-prove-range "$from" "$from" "$B" handover "$R/n_$name" > "$R/n_$name.log" 2>&1
    local rc=$?
    if [ $rc != 0 ] && /usr/bin/grep -aq "$want" "$R/n_$name.log"; then pass "reject $name: $want"; else
        wrong "reject $name (rc=$rc): want '$want'; got: $(/usr/bin/grep -a 'panicked\|failed' "$R/n_$name.log" | tail -1 | cut -c1-160)"; fi
}
neg self_id     "not the predecessor's"
neg profile     "handover: profile mismatch"
neg lineage     "predecessor lineage mismatch"
neg forest      "block forest"
neg kind        "not a ChainState"
neg height      "handover: height" $((LAST-1))
neg wrong_image "predecessor chain proof invalid\|no receipt found\|assumption"
neg no_receipt  "predecessor chain proof invalid\|no receipt found\|assumption"

# An image with no predecessor has no handover step.
HAZYNC_TEST_ISBASE2=1 timeout 900 "$O" libzync-prove-range 1 1 "$B" genesis "$R/o_b2" > "$R/o_b2.log" 2>&1
if [ $? != 0 ] && /usr/bin/grep -aq "this image has no predecessor" "$R/o_b2.log"; then pass "O refuses is_base = 2"; else wrong "O and is_base = 2"; fi
# A handover at a height the pin does not name must not be reachable through the host either.
HAZYNC_HANDOVER_RECEIPT=$R/o/block_$((LAST-1)).rcpt timeout 300 "$N" libzync-prove-range $LAST $LAST "$B" handover "$R/n_h" > "$R/n_h.log" 2>&1
if [ $? != 0 ] && /usr/bin/grep -aq "the pin hands over at" "$R/n_h.log"; then pass "host refuses a receipt at the wrong height"; else wrong "host and a wrong-height receipt"; fi

# The control: the same step with nothing tampered, then ordinary recursion.
HAZYNC_HANDOVER_RECEIPT=$R/o/block_$LAST.rcpt "$N" libzync-prove-range "$FIRST" "$END" "$B" handover "$R/n" > "$R/n.log" 2>&1 || { tail -5 "$R/n.log"; wrong "N continues $FIRST..$END"; }
[ -s "$R/n/state_$END.json" ] && pass "N continues $FIRST..$END by a handover"
/usr/bin/grep -aq "handover: verifies predecessor" "$R/n.log" && pass "block $FIRST ran as a handover step" || wrong "block $FIRST was not a handover step"
[ "$(/usr/bin/grep -ac '(verifies prev)' "$R/n.log")" -eq $((END-FIRST)) ] && pass "every block after the handover ran as ordinary recursion" || wrong "every block after the handover ran as ordinary recursion"

python3 - "$R" "$LAST" "$FIRST" "$END" "$B" "$REF" "$(cat "$R/o_id")" "$(cat "$R/n_id")" <<'P'
import importlib.util, json, sys, struct, hashlib
R, last, first, end, B, ref, oid, nid = sys.argv[1], *map(int, sys.argv[2:5]), *sys.argv[5:]
s = importlib.util.spec_from_file_location("ref", ref); m = importlib.util.module_from_spec(s); s.loader.exec_module(m)
a, b, c = (json.load(open(f"{R}/{d}/state_{h}.json")) for d, h in (("o", last), ("n", first), ("n", end)))
ow = list(struct.unpack("<8I", bytes.fromhex(oid))); nw = list(struct.unpack("<8I", bytes.fromhex(nid)))
def t(ok, what):
    print(("PASS " if ok else "WRONG ") + what)
    return ok
ok = True
ok &= t(a["self_id"] == ow and a["lineage"] == [0] * 32, "O's state: its own id, zero lineage")
ok &= t(b["self_id"] == nw and c["self_id"] == nw, "after the handover the state carries N's id")
ok &= t(a["anchor_id"] == b["anchor_id"] == c["anchor_id"], "anchor_id unchanged across the handover")
want = list(m.lineage([(ow, first)]))
ok &= t(b["lineage"] == want == c["lineage"], "lineage = the reference's digest of [O at first]; unchanged by later steps")
hs = [bytes.fromhex(l.strip())[::-1] for l in open(f"{R}/hashes.txt")]
for st, h in ((a, last), (c, end)):
    got = [bytes(r) if r else None for r in st["block_roots"]]
    ok &= t(got == m.roots(hs[:h + 1]) and st["block_leaves"] == h + 1, f"block forest at {h} == the reference over real block hashes")
    p = m.path(hs[:h + 1], 7)
    node, pos = m.leaf(hs[7], 7), 7
    tree = next(th for th, s0 in m.trees(h + 1) if s0 <= 7 < s0 + (1 << th))
    pos -= next(s0 for th, s0 in m.trees(h + 1) if th == tree)
    for sib in p:
        node = m.parent(node, sib) if pos & 1 == 0 else m.parent(sib, node)
        pos >>= 1
    ok &= t(node == got[tree], f"block 7 checks against the state at {h}")
sys.exit(0 if ok else 1)
P
[ $? = 0 ] && pass "state checks" || wrong "state checks"
echo "handover run.sh: $((N_CASES-BAD))/$N_CASES as expected"
[ "$BAD" = 0 ]
