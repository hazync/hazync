#!/bin/bash
# verify_controls.sh <ts-verify> <ts-verify, controls build> <anchored receipt block_105.rcpt> <anchor_104.json> <work dir>
# -- EXPERIMENTAL (TRUSTED_STATE.md §14.3). The prototype verifier on REAL receipts of the regtest harness: main@104
# (scripts/zapp-regtest/receipts/a/block_104.rcpt, genesis-anchored) and the anchored lineage's block 105, a real proof
# (core image eb2cd424, base step on the anchor at 104; §14.1). Each check that turns ANCHORED into something else is
# driven on the honest inputs with one thing changed. Prints VERIFY_CONTROLS_OK or VERIFY_CONTROLS_FAILED <n>.
set -uo pipefail
V=$(readlink -f "$1"); VC=$(readlink -f "$2"); R=$(readlink -f "$3"); A=$(readlink -f "$4"); W=$5
Z=$(cd "$(dirname "$0")/../.." && pwd); M=$Z/scripts/zapp-regtest/receipts/a/block_104.rcpt
mkdir -p "$W"; cd "$W" || exit 1
unset RISC0_DEV_MODE
bad=0
# want <label> <exit code> <verdict prefix> <command...>
want() {
  local label=$1 code=$2 pre=$3; shift 3
  local o rc; o=$("$@" 2>&1); rc=$?
  local v; v=$(echo "$o" | grep -a "^TS_VERIFY_" | tail -1)
  case "$rc:$v" in "$code:$pre"*) echo "PASS $label: exit $rc, ${v:0:150}";; *) echo "FAIL $label: exit $rc, '${v:0:200}', wanted $code $pre"; bad=$((bad+1));; esac
}
want "main@104, genesis-anchored"                    0 TS_VERIFY_OK       "$V" --network regtest "$M"
want "anchored block 105 + the pinned anchor"        3 TS_VERIFY_ANCHORED "$V" --network regtest --anchor "$A" "$R"
want "anchored block 105, no anchor given"           1 TS_VERIFY_FAIL     "$V" --network regtest "$R"
python3 - "$A" a_time.json <<'PYEOF'
import json, sys
j = json.load(open(sys.argv[1])); j["max_time"] += 1; json.dump(j, open(sys.argv[2], "w"))
PYEOF
want "anchor with max_time + 1 (not the proof's)"    1 TS_VERIFY_FAIL     "$V" --network regtest --anchor a_time.json "$R"
python3 - "$R" r_bad.rcpt <<'PYEOF'
import sys
b = bytearray(open(sys.argv[1], "rb").read()); b[len(b) // 2] ^= 1; open(sys.argv[2], "wb").write(b)
PYEOF
want "anchored receipt, one seal byte flipped"       1 TS_VERIFY_FAIL     "$V" --network regtest --anchor "$A" r_bad.rcpt
want "main@104 given as if anchored on 104"          1 TS_VERIFY_FAIL     "$V" --network regtest --anchor "$A" "$M"
want "real build refuses the controls hook"          1 TS_VERIFY_FAIL     env TS_VERIFY_REGTEST_PIN=pending "$V" --network regtest --anchor "$A" "$R"
want "controls build, pin unchanged"                 3 TS_VERIFY_ANCHORED "$VC" --network regtest --anchor "$A" "$R"
want "controls build, regtest pin PENDING"           2 "TS_VERIFY_DEMO: the regtest anchor at 104 is PENDING" \
     env TS_VERIFY_REGTEST_PIN=pending "$VC" --network regtest --anchor "$A" "$R"
want "controls build, another anchor_id pinned"      2 "TS_VERIFY_DEMO: the anchor's link digest" \
     env TS_VERIFY_REGTEST_PIN=00000000000000000000000000000000000000000000000000000000000000aa "$VC" --network regtest --anchor "$A" "$R"
[ "$bad" = 0 ] && echo VERIFY_CONTROLS_OK || { echo "VERIFY_CONTROLS_FAILED $bad"; exit 1; }
