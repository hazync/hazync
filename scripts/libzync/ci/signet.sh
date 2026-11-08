#!/bin/bash
# libzync CI: signet 1..100 from genesis on the published signet image (dev mode), the coin log and block forest
# against the independent implementations (signet's own consensus: the BIP325 block solution is checked per block).
# usage: signet.sh <signet host> <out dir>
. "$(dirname "$0")/lib.sh"
H=$(readlink -f "${1:?signet host}"); O=${2:?out dir}; mkdir -p "$O"; O=$(readlink -f "$O")
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=${HAZYNC_HINT_THREADS:-2}
$H method-id | head -1
step "signet 1..100 from genesis"
G=$O/s100; rm -rf "$G"
$H libzync-prove-range 1 100 "$T/signet" genesis "$G" > "$O/s100.log" 2>&1; echo "exit $?"
want "$O/s100.log" "LZ_PROVE_RANGE_DONE 1..=100" "prove-range 1..100 finished"
none "$O/s100.log" "panicked" "no guest or host panic"
STATES_OUT=$O/s100_states.jsonl python3 "$Z/scripts/libzync/coinlog_ref.py" "$T/signet" 100 "$G/state_100.json" > "$O/coinlog_ref.log" 2>&1
want "$O/coinlog_ref.log" "COINLOG_REF_MATCH" "coin log == coinlog_ref.py"
python3 "$Z/scripts/libzync/block_forest_ref.py" check "$T/signet/hashes.txt" "$O/s100_states.jsonl" "$G/state_100.json" > "$O/forest_ref.log" 2>&1
want "$O/forest_ref.log" "BLOCK_FOREST_OK" "block forest == block_forest_ref.py"
finish signet
