#!/bin/bash
# Coin log v2 + BIP30 tree counts tracked coins only (11-03): every test, edge blocks, signet.
R=~/libzync-gpu/r18; O=~/libzync-gpu/c7; Z=~/dev/hazync-libzync; CR=$Z/experimental/libzync-bench/core_rules; S=$Z/scripts/libzync
step() { echo; echo "== $*"; }
build() { bash $R/build.sh "$@"; grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_$1.log; [ -x $R/host_$1 ] || { echo "BUILD_FAILED $1"; exit 1; }; }
[ -x $R/host_c7m ] || { step "build mainnet (c7m)"; build c7m HAZYNC_COINLOG=1; }
step "build regtest O (c7reg)"; build c7reg HAZYNC_NETWORK=regtest HAZYNC_COINLOG=1
step "rig vs Core 28.0, coin log"
rm -rf $O/rig; mkdir -p $O/rig
RIG_COINLOG=1 BITCOIND=$HOME/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind PYTHONUNBUFFERED=1 RIG_VERBOSE=1 python3 $CR/rig.py $R/host_c7reg $O/rig > $O/rig.out 2>&1; echo RIG_EXIT=$?
grep -a '^[A-Z-]\{4,\} ' $O/rig.out | grep -v '^AGREE'; tail -1 $O/rig.out
step "build regtest N: O pinned at 101 (pin not committed)"
P=$Z/prover/methods/guest/src/lineage/regtest_coinlog.rs; cp $P $O/lineage.rs.orig
W=$($R/host_c7reg method-id 2>/dev/null | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[crate::handover_pin::Handover\] = &\[\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[crate::handover_pin::Handover { image: [$W], first_height: 101 }];/" $P
grep -q 'first_height: 101' $P || { echo PIN_EDIT_FAILED; cp $O/lineage.rs.orig $P; exit 1; }
bash $R/build.sh c7regN HAZYNC_NETWORK=regtest HAZYNC_COINLOG=1; cp $O/lineage.rs.orig $P
grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_c7regN.log; [ -x $R/host_c7regN ] || { echo BUILD_FAILED N; exit 1; }
( cd $Z && git diff --quiet -- prover/methods/guest/src/lineage/regtest_coinlog.rs ) && echo "pin reverted" || echo "PIN NOT REVERTED"
step "handover O -> N"
rm -rf $O/ho; mkdir -p $O/ho; $R/host_c7reg method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/o_id; $R/host_c7regN method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/n_id
bash $Z/experimental/libzync-bench/handover/run.sh $R/host_c7reg $R/host_c7regN $O/rig/blocks/main $O/ho 2>&1 | grep -a 'PASS\|WRONG\|as expected'; echo HO_EXIT=${PIPESTATUS[0]}
step "build signet (c7sig)"; build c7sig HAZYNC_NETWORK=signet HAZYNC_COINLOG=1
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
step "mainnet 1..200 from genesis (dev mode) vs coinlog_ref.py, block_forest_ref.py, genesis anchor"
rm -rf $O/m200; $R/host_c7m libzync-prove-range 1 200 $HOME/libzync-gpu/ssweep genesis $O/m200 > $O/m200.log 2>&1; echo PR_EXIT=$?; grep -a 'LZ_PROVE_RANGE_DONE\|panicked' $O/m200.log | grep -v 'collect run' | tail -2
STATES_OUT=$O/m200_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/ssweep 200 $O/m200/state_200.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_200.txt $O/m200_states.jsonl $O/m200/state_200.json | tail -1
python3 $S/genesis_anchor_id.py mainnet $(printf '0%.0s' {1..64}) | tail -1; python3 -c "import json;s=json.load(open('$O/m200/state_200.json'));print('state anchor_id', bytes(s['anchor_id']).hex(), 'version', s['version'], 'profile', hex(s['profile']))"
step "cost on real blocks: coin log (c7m) vs design B (r17m), execute mode, 16 chunks"
unset RISC0_DEV_MODE; export HAZYNC_CHUNKS=16 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
mkdir -p $O/cost
for b in $Z/prover/block_962000.json $HOME/libzync-gpu/sample/block_{970000,900000,850000,800000,750000,700000,650000}.json $R/exc/block_170060.json $R/exc/block_692261.json; do
  n=$(basename $b .json)
  for img in r17m c7m; do
    HAZYNC_BLOCK=$b $R/host_$img libzync-hints $O/cost/${n}_$img.lzb > /dev/null 2>&1
    HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/${n}_$img.lzb $R/host_$img agg-exec-fake > $O/cost/${n}_$img.log 2>&1
    awk -v n=$n -v i=$img '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "%-14s %-5s chunks %12d aggregate %11d whole %12d %s\n", n, i, c, a, c+a, (bad?"INVALID":"ok")}' $O/cost/${n}_$img.log
  done
done
step "edge blocks (round 3): BIP30 repeats, activation boundaries, first retarget, the largest legacy tx; coin log (c7m), modelled log"
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  b=$HOME/libzync-gpu/edge/block_$h.json
  HAZYNC_BLOCK=$b $R/host_c7m libzync-hints $O/cost/edge_$h.lzb > /dev/null 2>&1
  HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/edge_$h.lzb timeout 3600 $R/host_c7m agg-exec-fake > $O/cost/edge_$h.log 2>&1; rc=$?
  awk -v n=$h -v rc=$rc '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "edge %-7s chunks %12d aggregate %11d whole %12d %s (exit %s)\n", n, c, a, c+a, (bad?"INVALID":(a>0?"ok":"NO RESULT")), rc}' $O/cost/edge_$h.log
done
export RISC0_DEV_MODE=1 HAZYNC_CHUNKS=1
step "signet 1..2600 from genesis (dev mode) vs coinlog_ref.py, block_forest_ref.py"
rm -rf $O/s2600; $R/host_c7sig libzync-prove-range 1 2600 $HOME/libzync-gpu/signet/json genesis $O/s2600 > $O/s2600.log 2>&1; echo PR_EXIT=$?; grep -a 'LZ_PROVE_RANGE_DONE\|panicked' $O/s2600.log | grep -v 'collect run' | tail -2
STATES_OUT=$O/s2600_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/signet/json 2600 $O/s2600/state_2600.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_2600.txt $O/s2600_states.jsonl $O/s2600/state_2600.json | tail -1
echo RETEST_DONE
