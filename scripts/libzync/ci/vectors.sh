#!/bin/bash
# libzync CI: Core 28's script_tests / tx_valid / tx_invalid (cases made by core_vectors.py from the Core tree the guest
# is built from), one shard of them, on the published mainnet image (execute mode):
#   - every case, the libsecp path and the batched path with honest hints: each agrees with Core
#   - the base cases, four forged-hint variants (all valid, opposite parity, x = r + n, mixed): nothing Core rejects passes
# Shard s of k takes cases s, s+k, s+2k, ... of each file, so the k shards cover every case once. Each run must report
# exactly its shard's count, so a shard that ran short cannot pass.
# usage: vectors.sh <mainnet host> <core_vectors.json> <core_vectors_base.json> <out dir> <shard> <of>
. "$(dirname "$0")/lib.sh"
H=$(readlink -f "${1:?mainnet host}"); CV=$(readlink -f "${2:?vectors}"); CVB=$(readlink -f "${3:?base vectors}")
O=${4:?out dir}; S=${5:?shard}; K=${6:?of}; mkdir -p "$O"; O=$(readlink -f "$O")
"$H" method-id 2>/dev/null | head -1

step "0. shard $S of $K"
for f in "$CV" "$CVB"; do
  python3 - "$f" "$O/$(basename "$f" .json)_$S.json" "$S" "$K" <<'P'
import json, sys
src, dst, s, k = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
c = json.load(open(src)); assert len(c) > 0 and 0 <= s < k
json.dump(c[s::k], open(dst, "w")); print(f"{dst}: {len(c[s::k])} of {len(c)}")
P
done
SCV=$O/$(basename "$CV" .json)_$S.json; SCVB=$O/$(basename "$CVB" .json)_$S.json
n() { python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))))" "$1"; }
NCV=$(n "$SCV"); NCVB=$(n "$SCVB")
if [ "$NCV" -gt 0 ] && [ "$NCVB" -gt 0 ]; then pass "shard $S: $NCV cases, $NCVB base cases"; else fail "shard $S is empty"; fi

# run <name> <cases> <args...>: in the background, two at a time (a runner has two cores)
run() { local m=$1 f=$2; shift 2; "$H" core-vectors "$f" 500 "$@" > "$O/vectors_$m.log" 2>&1; echo "exit $?" >> "$O/vectors_$m.log"; }
check() { # check <name> <expected count> <what>
  local L; L=$(grep -a '^CORE_VECTORS' "$O/vectors_$1.log" | tail -1); echo "$L"
  if echo "$L" | grep -q ", 0 differ, of $2 "; then pass "vectors [$1]: $3 ($2 cases)"; else fail "vectors [$1]: '$L' (want 0 differ, of $2)"; tail -3 "$O/vectors_$1.log" | sed 's/^/    /'; fi
}

step "1. every case: libsecp and honest hints"
run libsecp "$SCV" & run honest "$SCV" --sig honest & wait
check libsecp "$NCV" "every case agrees with Core"
check honest "$NCV" "every case agrees with Core"

step "2. base cases: forged hints"
run forged "$SCVB" --sig forged & run forged-odd "$SCVB" --sig forged-odd & wait
run forged-rn "$SCVB" --sig forged-rn & run forged-mixed "$SCVB" --sig forged-mixed & wait
for v in forged forged-odd forged-rn forged-mixed; do check $v "$NCVB" "nothing Core rejects passes"; done
finish "vectors shard $S/$K"
