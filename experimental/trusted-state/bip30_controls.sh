#!/bin/bash
# bip30_controls.sh <ts-anchor> <ts-ingest-host> <ts-ingest-host, controls build> <work dir> -- EXPERIMENTAL
# The split BIP30 receipt (docs/libzync/TRUSTED_STATE.md §13.1) on the regtest snapshot at 104 (100 BIP30 entries + 1
# REPEATABLE): the chunks (3) executed once, the tree inline and in two splits (depth 4 / 3 pieces, depth 8 / 5 pieces),
# then the controls. Every control is driven BOTH ways: refused by the real images, and accepted (with a combined
# journal that is NOT the honest one) by the controls build with exactly its check cut (TS_CUT), so each refusal is
# shown to come from that check. Prints BIP30_CONTROLS_OK or BIP30_CONTROLS_FAILED <n>.
set -uo pipefail
TS=$1; IH=$2; IHC=$3; W=$4
Z=$(cd "$(dirname "$0")/../.." && pwd); G=$Z/scripts/zapp-regtest
mkdir -p "$W"; cd "$W" || exit 1
export RISC0_DEV_MODE=1
# cut bits (ingest/tsi.rs)
BOUNDS=1; RANGE=2; ORDER=4; TILE=8; ECHAIN=16; ECOUNT=32; RCHAIN=64
python3 "$Z/experimental/trusted-state/side_inputs.py" "$G/blocks/main" 104 side > /dev/null
rm -rf A S1 S2 X; env -u RISC0_DEV_MODE "$TS" chunk-plan --network regtest --snapshot "$G/oracle/utxo_104.dat" --side side --chunks 3 --out A | head -1
for f in A/chunk_*.in; do "$IH" chunk "$f" "${f%.in}.rcpt" > "${f%.in}.log" 2>&1 || { echo "chunk $f failed"; exit 1; }; done
# the controls build's images have their own ids (another target directory), so it gets its own chunk receipts
for f in A/chunk_*.in; do "$IHC" chunk "$f" "${f%.in}.c.rcpt" > /dev/null 2>&1 || { echo "controls chunk $f failed"; exit 1; }; done
C=(A/chunk_0.rcpt A/chunk_1.rcpt A/chunk_2.rcpt)
CC=(A/chunk_0.c.rcpt A/chunk_1.c.rcpt A/chunk_2.c.rcpt)
"$TS" bip30-plan --final A/final.in --depth 4 --pieces 3 --out S1
"$TS" bip30-plan --final A/final.in --depth 8 --pieces 5 --out S2
mkdir -p X
# crafted piece inputs (python): sections [d,lo,hi], e_in, r_in, entries (36 B), REPEATABLE (32 B)
python3 - <<'PYEOF'
import hashlib, struct
def rd(p):
    b = open(p, "rb").read(); i = 0; s = []
    while i < len(b):
        n = int.from_bytes(b[i:i + 4], "little"); s.append(b[i + 4:i + 4 + n]); i += 4 + n
    return s
def wr(p, s): open(p, "wb").write(b"".join(len(x).to_bytes(4, "little") + x for x in s))
def step(c, k, v): return hashlib.sha256(c + k + struct.pack("<I", v)).digest()
REP = 0xFFFFFFFE
P = [rd(f"S1/piece_{i}.in") for i in range(3)]
ents = lambda s: [s[3][i:i + 36] for i in range(0, len(s[3]), 36)]
# 1. RANGE: the first entry of piece 1 moved to the end of piece 0 (piece 0's range unchanged); piece 1 starts after it
p0, p1 = [list(x) for x in P[0]], [list(x) for x in P[1]]
e1 = ents(P[1]); moved = e1[0]
a = list(P[0]); a[3] = P[0][3] + moved; wr("X/range_0.in", a)
b = list(P[1]); b[1] = step(P[1][1], moved[:32], int.from_bytes(moved[32:], "little")); b[3] = b"".join(e1[1:]); wr("X/range_1.in", b)
# 2. ORDER: a REPEATABLE txid that has live coins (piece 1's first entry's txid added to its REPEATABLE list); later
#    pieces' REPEATABLE chains continue from the changed one
b = list(P[1]); reps = sorted([b[4][i:i + 32] for i in range(0, len(b[4]), 32)] + [e1[0][:32]]); b[4] = b"".join(reps); wr("X/dup_1.in", b)
rc = P[1][2]
for k in reps: rc = step(rc, k, REP)
c = list(P[2]); c[2] = rc; wr("X/dup_2.in", c)
# 2b. ORDER: piece 1's first and last entries swapped (the chain then follows the swapped order); later pieces unchanged
b = list(P[1]); sw = e1[-1:] + e1[1:-1] + e1[:1]; b[3] = b"".join(sw); wr("X/swap_1.in", b)
# 3. ECHAIN/ECOUNT: one entry left out of piece 1 (the host omits it); piece 2 starts after the full chain as before
b = list(P[1]); b[3] = b"".join(e1[1:]); wr("X/drop_1.in", b)
# 4. BOUNDS: piece 2 claiming prefixes up to 17 of a depth-4 tree
c = list(P[2]); c[0] = struct.pack("<III", *struct.unpack("<III", P[2][0])[:2], 17); wr("X/bounds_2.in", c)
print("crafted: range_0/1 (moved entry), dup_1/2 (REPEATABLE with live coins), drop_1 (an entry omitted), bounds_2 (p_hi 17 at d 4)")
PYEOF
bad=0
piece() {  # piece <host> <cut> <in> <rcpt>: run one piece, 0 if it executed
  TS_CUT=$2 "$1" bip30 "$3" "$4" > "${4%.rcpt}.log" 2>&1
}
for s in S1 S2; do for f in $s/piece_*.in; do piece "$IH" 0 "$f" "${f%.in}.rcpt" || { echo "honest piece $f refused"; exit 1; }; grep -h "^BIP30\|TSB" "${f%.in}.log"; done; done
for f in S1/piece_*.in; do piece "$IHC" 0 "$f" "${f%.in}.c.rcpt" || { echo "controls-build piece $f refused"; exit 1; }; done
combine() {  # combine <host> <cut> <expected> <args...>: prints ok/refused, and MATCH/MISMATCH when ok
  local h=$1 cut=$2 exp=$3; shift 3
  local o; o=$(TS_CUT=$cut "$h" combine A/final.in "$exp" "$@" 2>&1)
  if echo "$o" | grep -aq "^COMBINE image"; then
    if echo "$o" | grep -aq COMBINE_MATCH; then echo "ok MATCH"; else echo "ok MISMATCH"; fi
  else echo "refused $(echo "$o" | grep -a -o "panicked.*\|Guest panicked: .*" | tail -1 | sed 's/.*Guest panicked: //' | cut -c1-110)"; fi
}
want() {  # want <label> <expected prefix> <got>
  case "$3" in "$2"*) echo "PASS $1: $3";; *) echo "FAIL $1: got '$3', wanted '$2'"; bad=$((bad+1));; esac
}
E=A/expected.journal
want "inline tree (§12 combine)"                 "ok MATCH" "$(combine "$IH" 0 $E "${C[@]}")"
want "split d=4, 3 pieces"                       "ok MATCH" "$(combine "$IH" 0 $E "${C[@]}" --bip30 S1/piece_{0,1,2}.rcpt)"
want "split d=8, 5 pieces"                       "ok MATCH" "$(combine "$IH" 0 $E "${C[@]}" --bip30 S2/piece_{0,1,2,3,4}.rcpt)"
want "controls build, no cut, split d=4"         "ok MATCH" "$(combine "$IHC" 0 $E "${CC[@]}" --bip30 S1/piece_{0,1,2}.c.rcpt)"
# combine-level: honest pieces, wrong sets
for c in "dropped piece|0 2|$((TILE|ECHAIN|ECOUNT|RCHAIN))" "reordered pieces|1 0 2|$((TILE|ECHAIN|RCHAIN))" "repeated piece|0 1 1 2|$((TILE|ECHAIN|ECOUNT|RCHAIN))"; do
  IFS='|' read -r label idx cut <<< "$c"
  r=(); rc=(); for i in $idx; do r+=("S1/piece_$i.rcpt"); rc+=("S1/piece_$i.c.rcpt"); done
  want "$label: real images"            "refused" "$(combine "$IH" 0 $E "${C[@]}" --bip30 "${r[@]}")"
  want "$label: controls, no cut"       "refused" "$(combine "$IHC" 0 $E "${CC[@]}" --bip30 "${rc[@]}")"
  want "$label: controls, cut $cut"     "ok MISMATCH" "$(combine "$IHC" "$cut" $E "${CC[@]}" --bip30 "${rc[@]}")"
done
# a piece of the depth-8 split in a depth-4 set
want "mixed depths: real images"         "refused" "$(combine "$IH" 0 $E "${C[@]}" --bip30 S1/piece_0.rcpt S2/piece_1.rcpt S1/piece_2.rcpt)"
# piece-level: crafted inputs
pc() {  # pc <label> <cut at the piece> <cut at the combine> <want under the cut> <crafted pieces: idx=file ...>
  local label=$1 pcut=$2 ccut=$3 wcut=$4; shift 4
  local real=(S1/piece_0.in S1/piece_1.in S1/piece_2.in) refused=no
  for kv in "$@"; do real[${kv%%=*}]=${kv#*=}; done
  local rr=() rcc=()
  for i in 0 1 2; do
    piece "$IH" 0 "${real[$i]}" "X/r$i.rcpt" || refused=yes; rr+=("X/r$i.rcpt")
  done
  if [ $refused = yes ]; then echo "PASS $label: real images: the piece is refused ($(grep -a -o 'panicked.*' X/r*.log | tail -1 | cut -c1-90))"
  else want "$label: real images" "refused" "$(combine "$IH" 0 $E "${C[@]}" --bip30 "${rr[@]}")"; fi
  for i in 0 1 2; do
    if ! piece "$IHC" "$pcut" "${real[$i]}" X/c$i.rcpt; then
      want "$label: controls, cut piece $pcut" "$wcut" "piece-refused ($(grep -a -o 'panicked.*' X/c$i.log | tail -1 | cut -c1-90))"; return
    fi
    rcc+=("X/c$i.rcpt")
  done
  want "$label: controls, cut piece $pcut / combine $ccut" "$wcut" "$(combine "$IHC" "$ccut" $E "${CC[@]}" --bip30 "${rcc[@]}")"
}
pc "entry outside its piece's range"    $RANGE  0                  "ok MISMATCH" 0=X/range_0.in 1=X/range_1.in
pc "entries out of order in a piece"    $ORDER  $ECHAIN            "ok MISMATCH" 1=X/swap_1.in
pc "an entry omitted from a piece"      0       $((ECHAIN|ECOUNT)) "ok MISMATCH" 1=X/drop_1.in
pc "prefix range beyond 2^d"            $BOUNDS $TILE              "ok MISMATCH" 2=X/bounds_2.in
# a REPEATABLE txid with live coins: a key twice. With the check cut the piece still aborts (smt_bulk runs past depth
# 256 on two equal keys), so this refusal does not rest on the order check alone
pc "REPEATABLE txid with live coins"    $ORDER  0                  "piece-refused" 1=X/dup_1.in 2=X/dup_2.in
[ $bad = 0 ] && echo BIP30_CONTROLS_OK || { echo "BIP30_CONTROLS_FAILED $bad"; exit 1; }
