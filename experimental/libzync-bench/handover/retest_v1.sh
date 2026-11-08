#!/bin/bash
# HISTORICAL (before e292c9d): edits handover_pin.rs, where the lineage lived then. Use retest_c5.sh.
# Full re-test of libzync after the 11-02 format change (version, utxo_value, survivors sorted by (txid, vout)).
R=~/libzync-gpu/r18; Z=~/dev/hazync-libzync; O=~/libzync-gpu/r17; CR=$Z/experimental/libzync-bench/core_rules
step() { echo; echo "== $*"; }
build() { bash $R/build.sh "$@"; grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_$1.log; [ -x $R/host_$1 ] || { echo "BUILD_FAILED $1"; exit 1; }; }
step "build regtest O (r17reg)"; build r17reg HAZYNC_NETWORK=regtest
step "rig vs Core 28.0 (fresh dir)"
rm -rf $O/rig; mkdir -p $O/rig
BITCOIND=$HOME/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind PYTHONUNBUFFERED=1 RIG_VERBOSE=1 python3 $CR/rig.py $R/host_r17reg $O/rig > $O/rig.out 2>&1; echo RIG_EXIT=$?
grep -a '^[A-Z-]\{4,\} ' $O/rig.out | grep -v '^AGREE' ; tail -1 $O/rig.out
step "build regtest N: O pinned at 101 (pin not committed)"
P=$Z/prover/methods/guest/src/handover_pin.rs; cp $P $O/handover_pin.rs.orig
W=$($R/host_r17reg method-id 2>/dev/null | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[Handover\] = &\[\];/pub const LINEAGE: \&[Handover] = \&[Handover { image: [$W], first_height: 101 }];/" $P
grep -q 'first_height: 101' $P || { echo PIN_EDIT_FAILED; cp $O/handover_pin.rs.orig $P; exit 1; }
bash $R/build.sh r17regN HAZYNC_NETWORK=regtest; cp $O/handover_pin.rs.orig $P
grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_r17regN.log; [ -x $R/host_r17regN ] || { echo BUILD_FAILED N; exit 1; }
( cd $Z && git diff --quiet -- prover/methods/guest/src/handover_pin.rs ) && echo "pin reverted" || echo "PIN NOT REVERTED"
step "handover run.sh O -> N"
rm -rf $O/ho; mkdir -p $O/ho; $R/host_r17reg method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/o_id; $R/host_r17regN method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/n_id; bash $Z/experimental/libzync-bench/handover/run.sh $R/host_r17reg $R/host_r17regN $O/rig/blocks/main $O/ho 2>&1 | tail -30; echo RUN_EXIT=${PIPESTATUS[0]}
step "build signet (r17sig)"; build r17sig HAZYNC_NETWORK=signet
step "SwiftSync regression (r17m + r17sig)"
REG_HOST=$R/host_r17m REG_HOST_SIG=$R/host_r17sig REG_OUT=$O/reg bash $CR/swiftsync_regress.sh 2>&1 | grep -av 'panicked at src/main.rs'; echo REG_EXIT=$?
step "block forest of the sweep states vs block_forest_ref.py"
for pair in "$HOME/libzync-gpu/ssweep 200 $O/reg/s200/state_200.json" "$HOME/libzync-gpu/signet/json 2600 $O/reg/sig/state_2600.json"; do
  set -- $pair
  python3 - $1 $2 $O/hashes_$2.txt <<'P'
import hashlib, json, struct, sys
B, end, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
hs = [json.load(open(f"{B}/block_1.json"))["prev"]]
for h in range(1, end + 1):
    j = json.load(open(f"{B}/block_{h}.json"))
    assert j["prev"] == hs[-1], h
    hdr = struct.pack("<i", j["version"]) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1] + struct.pack("<III", j["time"], j["bits"], j["nonce"])
    hs.append(hashlib.sha256(hashlib.sha256(hdr).digest()).digest()[::-1].hex())
open(out, "w").write("\n".join(hs) + "\n")
P
  python3 $Z/scripts/libzync/block_forest_ref.py check $O/hashes_$2.txt $3 | tail -1
done
step "utxo_value vs reference"
python3 $Z/scripts/libzync/utreexo_ref.py ~/libzync-gpu/ssweep 200 $O/reg/s200/state_200.json | sed -n '1p;$p'
python3 $Z/scripts/libzync/utreexo_ref.py ~/libzync-gpu/signet/json 2600 $O/reg/sig/state_2600.json | sed -n '1p;$p'
step "snapshot checker end to end (test_utxo_snapshot.py)"
python3 $Z/scripts/libzync/test_utxo_snapshot.py ~/libzync-gpu/ssweep 200 $O/reg/s200/state_200.json 2>&1 | tail -3
python3 $Z/scripts/libzync/test_utxo_snapshot.py ~/libzync-gpu/signet/json 2600 $O/reg/sig/state_2600.json 2>&1 | tail -3
echo RETEST_DONE
