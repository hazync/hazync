#!/bin/bash
# libzync CI: the chain from genesis on the MAINNET TEST VARIANT host (dev mode: every guest executed and its claims
# checked, no receipts proved), then every test that builds on it.
# The test variant (build-hosts.sh host_m0, HAZYNC_TEST_LINEAGE=1) is the mainnet coin-log image with an empty test
# lineage instead of the published one. The published image pins its predecessor below its handover height (2000 in
# v0.26.0), so by design it never proves those blocks: its prep stamps them with the predecessor's id, and a proof of
# one by it is refused as a forged history. Everything else is the same source.
#   1. mainnet 1..200 from genesis; the coin log and block forest equal two independent Python implementations; the
#      genesis anchor is the pinned one;
#   2. leaf bundles: prep 1..200 equals the proved run; witnesses equal the bundles; leaves 198..200 proved from their
#      bundles alone join with linear 197 to exactly the linear block 200;
#   3. joins: 200 leaves in a balanced tree = the linear block 200; mixed shapes; gap / swapped / twice / tampered
#      anchor refused by the guest;
#   4. prep continued under another image id re-stamps and stays byte-identical;
#   5. the coordinator in libzync mode with the stock worker: leaves proved, submitted, a range joined; verify-any with
#      a wrong start refused;
#   6. the board handover (experimental/libzync-bench/handover/board_rehearsal.sh, docs/HANDOVER.md §4): O = the test
#      variant, N = it pinned at 11 (build-hosts.sh host_mN), mainnet 1..20; and its no-stamp control, which must fail.
# usage: chain.sh <mainnet test-variant host O> <its N (O pinned at 11)> <out dir>
. "$(dirname "$0")/lib.sh"
H=$(readlink -f "${1:?mainnet test-variant host}"); HN=$(readlink -f "${2:?its N}"); O=${3:?out dir}; mkdir -p "$O"; O=$(readlink -f "$O")
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=${HAZYNC_HINT_THREADS:-2}
$H method-id
# from genesis needs an image with no predecessor: refuse the published host (its lineage) with the reason, not 15 FAILs
$H libzync-lineage > "$O/lineage.log" 2>&1
if ! grep -aq '^TEST_VARIANT ' "$O/lineage.log" || ! grep -aqx 'HANDOVER none' "$O/lineage.log"; then
  cat "$O/lineage.log"; echo "chain.sh: $1 is not the mainnet test variant with an empty lineage (build-hosts.sh host_m0)"; exit 1
fi

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
H=$H J=$T/mainnet O=$O/crash bash "$B/prep/crash_test.sh" > "$O/crash.log" 2>&1
want "$O/crash.log" "CRASH_TEST_OK" "a crash between the trie flush and its marker: restarts from the trie's height, coin log untouched (review 11-12 #3, #8)"
H=$H J=$T/mainnet O=$O/crash-c bash "$B/prep/crash_test.sh" --control > "$O/crash-c.log" 2>&1
want "$O/crash-c.log" "CONTROL OK" "crash test control: the script without the trie-height rule fails"

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
step "6. board handover rehearsal (O = the test variant, N = O pinned at 11, mainnet 1..20)"
# board_rehearsal.sh reads H as the handover HEIGHT (here H is the host O)
rehearse() { env H=11 END=20 J="$T/mainnet" D="$1" bash "$B/handover/board_rehearsal.sh" "$HO" "$HN" "${@:2}"; }
HO=$H
rehearse "$O/rehearsal" > "$O/rehearsal.log" 2>&1; rc=$?
L=$(grep -a '^board_rehearsal.sh: ' "$O/rehearsal.log" | tail -1); k=$(echo "$L" | sed -n 's#.*: \([0-9]*\)/\([0-9]*\) as expected#\1 \2#p')
if [ $rc = 0 ] && [ -n "$k" ] && [ "${k% *}" = "${k#* }" ] && [ "${k% *}" -ge 19 ]; then pass "rehearsal: $L"
else fail "rehearsal: '$L' (exit $rc)"; grep -a '^WRONG\|failed' "$O/rehearsal.log" | head -10 | sed 's/^/    /'; fi
rehearse "$O/rehearsal-c" --control > "$O/rehearsal-c.log" 2>&1; rc=$?
L=$(grep -a '^board_rehearsal.sh --control: ' "$O/rehearsal-c.log" | tail -1)
if [ $rc != 0 ] && [ -n "$L" ] && grep -aq '^WRONG N blocks verified above 11: ' "$O/rehearsal-c.log"; then
  pass "rehearsal control (no stamping): fails as it must, N's blocks above 11 never verify ($L)"
else fail "rehearsal control: '$L' (exit $rc; want it to fail at 'N blocks verified above 11')"; grep -a '^WRONG' "$O/rehearsal-c.log" | head -5 | sed 's/^/    /'; fi
finish chain
