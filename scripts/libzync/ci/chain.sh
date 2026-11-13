#!/bin/bash
# libzync CI: the chain from genesis on the MAINNET host (dev mode: every guest executed and its claims checked, no
# receipts proved), then every test that builds on it.
#   1. mainnet 1..200 from genesis; the coin log and block forest equal two independent Python implementations; the
#      genesis anchor is the pinned one;
#   2. leaf bundles: prep 1..200 equals the proved run; witnesses equal the bundles; leaves 198..200 proved from their
#      bundles alone join with linear 197 to exactly the linear block 200;
#   3. joins: 200 leaves in a balanced tree = the linear block 200; mixed shapes; gap / swapped / twice / tampered
#      anchor refused by the guest;
#   4. prep continued under another image id re-stamps and stays byte-identical;
#   5. the coordinator in libzync mode with the stock worker: leaves proved, submitted, a range joined; verify-any with
#      a wrong start refused.
# usage: chain.sh <mainnet host> <out dir>
. "$(dirname "$0")/lib.sh"
H=$(readlink -f "${1:?mainnet host}"); O=${2:?out dir}; mkdir -p "$O"; O=$(readlink -f "$O")
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=${HAZYNC_HINT_THREADS:-2}
$H method-id | head -1

step "1. mainnet 1..200 from genesis"
M=$O/m200; rm -rf "$M"
HAZYNC_COINLOG_SAVE_EACH=1 $H libzync-prove-range 1 200 "$T/mainnet" genesis "$M" > "$O/m200.log" 2>&1; echo "exit $?"
want "$O/m200.log" "LZ_PROVE_RANGE_DONE 1..=200" "prove-range 1..200 finished"
none "$O/m200.log" "panicked" "no guest or host panic"
STATES_OUT=$O/m200_states.jsonl python3 "$Z/scripts/libzync/coinlog_ref.py" "$T/mainnet" 200 "$M/state_200.json" > "$O/coinlog_ref.log" 2>&1
want "$O/coinlog_ref.log" "COINLOG_REF_MATCH" "coin log == coinlog_ref.py"
python3 "$Z/scripts/libzync/block_forest_ref.py" check "$T/mainnet/hashes.txt" "$O/m200_states.jsonl" "$M/state_200.json" > "$O/forest_ref.log" 2>&1
want "$O/forest_ref.log" "BLOCK_FOREST_OK" "block forest == block_forest_ref.py"
GA=$(python3 "$Z/scripts/libzync/genesis_anchor_id.py" mainnet "$(printf '0%.0s' {1..64})" | grep -oE 'anchor_id [0-9a-f]{64}' | cut -d' ' -f2)
SA=$(python3 -c "import json;print(bytes(json.load(open('$M/state_200.json'))['anchor_id']).hex())")
[ -n "$GA" ] && [ "$GA" = "$SA" ] && pass "state_200 anchor_id == the pinned genesis anchor ${GA:0:8}" || fail "anchor_id ${SA:0:16} != genesis ${GA:0:16}"

step "2. leaf bundles"
H=$H O=$O/leaf J=$T/mainnet REF=$M bash "$B/leaf/leaf_test.sh" > "$O/leaf.log" 2>&1
want "$O/leaf.log" "LEAF_TEST_DONE" "leaf test ran to the end"
want "$O/leaf.log" "prep EXIT=0" "prep 1..200 with bundles"
none "$O/leaf.log" "PREP_MISMATCH" "prep states == the proved run"
for h in 198 199 200; do want "$O/leaf.log" "leaf $h EXIT=0" "leaf $h proved from its bundle alone"; done
[ "$(grep -ac "differs only in \['anchor_id'\]" "$O/leaf.log")" = 3 ] && pass "each leaf state == linear but anchor_id" || fail "leaf states differ beyond anchor_id"
want "$O/leaf.log" ": EQUAL" "linear 197 + leaves 198..200 joined == linear block 200"
want "$O/leaf.log" "prove-range with HAZYNC_LEAF_CHECK EXIT=0" "every block's witnesses == its bundle's"
want "$O/leaf.log" "check run: state_200 ==" "the checked run's state_200 == the first run's"

step "3. joins"
J=$T/mainnet P=${P:-4} bash "$B/join/test_join.sh" "$H" "$M" "$O/join" 200 > "$O/join.log" 2>&1
want "$O/join.log" "JOIN_TEST_PASS" "join tree, mixed shape, 4 refused negatives"

step "4. prep re-stamped under another image"
H=$H J=$T/mainnet O=$O/restamp bash "$B/prep/restamp_test.sh" > "$O/restamp.log" 2>&1
want "$O/restamp.log" "RESTAMP_TEST_OK" "states and bundles byte-identical"

step "5. coordinator (libzync mode) + stock worker"
H=$H O=$O/coord J=$T/mainnet REF=$M C=$Z/coordinator bash "$B/coord/coord_test.sh" > "$O/coord.log" 2>&1
want "$O/coord.log" "COORD_TEST_DONE" "coordinator test ran to the end"
for x in "prove 1 EXIT=0" "submit 1 EXIT=0" "prove 2 EXIT=0" "submit 2 EXIT=0" "run 3-4 EXIT=0" "RANGE-OK"; do want "$O/coord.log" "$x" "$x"; done
want "$O/coord.log" "starting at 1 EXIT=1" "verify-any: block 2 claimed from 1 refused"
want "$O/coord.log" "starting at 4 EXIT=1" "verify-any: 3-4 claimed from 4 refused"
want "$O/coord.log" "fake without the test switch EXIT=1 (want 1): REJECT a dev-mode FakeReceipt" "verify-any: a fake receipt refused without HAZYNC_TEST_FAKE_RECEIPTS"
none "$O/coord.log" "starting at 1 EXIT=1 (want 1): REJECT a dev-mode" "block 2 from 1 is refused for its start, not as a fake"
none "$O/coord.log" "starting at 4 EXIT=1 (want 1): REJECT a dev-mode" "3-4 from 4 is refused for its start, not as a fake"
none "$O/coord.log" "Traceback" "no server traceback"
finish chain
