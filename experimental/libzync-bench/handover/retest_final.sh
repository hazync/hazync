#!/bin/bash
# ROADMAP H4: the full test pass on the product image (coin log + trie receipt + live-coin hash + join mode).
# Unit tests; rig vs Core 28.0 (regtest); handover O -> N; mainnet 1..200 vs the reference implementations; cost on
# real blocks (vs the previous image); edge blocks; signet 1..SN from genesis. Each step prints its own verdict line.
# usage: [SN=100] retest_final.sh <tag, e.g. c13: builds host_<tag>m / reg / regN / sig> <previous mainnet host tag, e.g. c10e12>
# SN (default 2600, the full signet chain the references cover): a short pass sets it lower when the change does not
# touch network rules or the long-chain coin log. Any 'panicked' line in a prove-range log is a failure.
T=${1:?tag}; PREV=${2:?previous mainnet host tag}; SN=${SN:-2600}
R=~/libzync-gpu/r18; O=~/libzync-gpu/$T; Z=~/dev/hazync-libzync; CR=$Z/experimental/libzync-bench/core_rules; S=$Z/scripts/libzync
B="HAZYNC_BASE=$HOME/hzb-r8"
mkdir -p $O
step() { echo; echo "== $*"; }
build() { bash $R/build.sh "$@"; grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_$1.log; [ -x $R/host_$1 ] || { echo "BUILD_FAILED $1"; exit 1; }; }
utest() {  # utest <name> <dir> <cargo args...>
  local n=$1 d=$2; shift 2
  ( cd $d && cargo test -j2 "$@" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > $O/ut_$n.out; echo "EXIT=${PIPESTATUS[0]}" >> $O/ut_$n.out )
  echo "$n: $(grep -a '^test result' $O/ut_$n.out | awk '{p+=$4; f+=$6} END{print p" passed, "f" failed"}') $(tail -1 $O/ut_$n.out)"
}
step "unit tests"
utest libzync $Z/libzync --release --features std
utest swiftsync-verify $Z/swiftsync-verify --release
utest swiftsync-verify-ffi $Z/swiftsync-verify-ffi --release
utest swiftsync-verify-wasm $Z/swiftsync-verify-wasm --release
RISC0_SKIP_BUILD=1 utest host $Z/prover -p host
grep -a '^test .* FAILED$' $O/ut_*.out
step "build mainnet (${T}m)"; build ${T}m $B
step "build regtest O (${T}reg)"; build ${T}reg $B HAZYNC_NETWORK=regtest
step "rig vs Core 28.0"
rm -rf $O/rig; mkdir -p $O/rig
RIG_COINLOG=1 BITCOIND=$HOME/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind PYTHONUNBUFFERED=1 RIG_VERBOSE=1 python3 $CR/rig.py $R/host_${T}reg $O/rig > $O/rig.out 2>&1; echo RIG_EXIT=$?
grep -a '^[A-Z-]\{4,\} ' $O/rig.out | grep -v '^AGREE'; tail -1 $O/rig.out
step "build regtest N: O pinned at 101 (pin not committed)"
P=$Z/prover/methods/guest/src/lineage/regtest_coinlog.rs; cp $P $O/lineage.rs.orig
W=$($R/host_${T}reg method-id 2>/dev/null | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[crate::handover_pin::Handover\] = &\[\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[crate::handover_pin::Handover { image: [$W], first_height: 101, verifier: crate::handover_pin::SAME_ZKVM }];/" $P
grep -q 'first_height: 101' $P || { echo PIN_EDIT_FAILED; cp $O/lineage.rs.orig $P; exit 1; }
bash $R/build.sh ${T}regN $B HAZYNC_NETWORK=regtest; cp $O/lineage.rs.orig $P
grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_${T}regN.log; [ -x $R/host_${T}regN ] || { echo BUILD_FAILED N; exit 1; }
( cd $Z && git diff --quiet -- prover/methods/guest/src/lineage/regtest_coinlog.rs ) && echo "pin reverted" || echo "PIN NOT REVERTED"
step "handover O -> N"
rm -rf $O/ho; mkdir -p $O/ho
$R/host_${T}reg method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/o_id; $R/host_${T}regN method-id 2>/dev/null | sed -n "s/^METHOD_ID //p" > $O/ho/n_id
bash $Z/experimental/libzync-bench/handover/run.sh $R/host_${T}reg $R/host_${T}regN $O/rig/blocks/main $O/ho 2>&1 | grep -a 'PASS\|WRONG\|as expected'; echo HO_EXIT=${PIPESTATUS[0]}
step "build signet (${T}sig)"; build ${T}sig $B HAZYNC_NETWORK=signet
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
step "mainnet 1..200 from genesis (dev mode) vs coinlog_ref.py, block_forest_ref.py, genesis anchor"
M=$O/m200; rm -rf $M; $R/host_${T}m libzync-prove-range 1 200 $HOME/libzync-gpu/ssweep genesis $M > $O/m200.log 2>&1; echo PR_EXIT=$?; grep -a 'LZ_PROVE_RANGE_DONE' $O/m200.log; echo "PANICS=$(grep -ac panicked $O/m200.log)"
STATES_OUT=$O/m200_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/ssweep 200 $M/state_200.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_200.txt $O/m200_states.jsonl $M/state_200.json | tail -1
python3 $S/genesis_anchor_id.py mainnet $(printf '0%.0s' {1..64}) | tail -1; python3 -c "import json;s=json.load(open('$M/state_200.json'));print('state anchor_id', bytes(s['anchor_id']).hex(), 'version', s['version'], 'profile', hex(s['profile']))"
step "cost on real blocks: $PREV vs ${T}m, execute mode, 16 chunks"
unset RISC0_DEV_MODE; export HAZYNC_CHUNKS=16 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
mkdir -p $O/cost
for b in $Z/prover/block_962000.json $HOME/libzync-gpu/sample/block_{970000,900000,850000,800000,750000,700000,650000}.json $R/exc/block_170060.json $R/exc/block_692261.json; do
  n=$(basename $b .json)
  for img in $PREV ${T}m; do
    # a failed hints step would leave a collect run (no hints): no cost figure, so it must not print ok
    HAZYNC_BLOCK=$b $R/host_$img libzync-hints $O/cost/${n}_$img.lzb > $O/cost/${n}_$img.hints.log 2>&1 || echo "HINTS_FAILED $n $img"
    HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/${n}_$img.lzb $R/host_$img agg-exec-fake > $O/cost/${n}_$img.log 2>&1
    awk -v n=$n -v i=$img '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "%-14s %-6s chunks %12d aggregate %11d whole %12d %s\n", n, i, c, a, c+a, (bad?"INVALID":(a>0?"ok":"NO RESULT"))}' $O/cost/${n}_$img.log
  done
done
step "edge blocks: BIP30 repeats, activation boundaries, first retarget, the largest legacy tx (${T}m, modelled log)"
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  b=$HOME/libzync-gpu/edge/block_$h.json
  HAZYNC_BLOCK=$b $R/host_${T}m libzync-hints $O/cost/edge_$h.lzb > $O/cost/edge_$h.hints.log 2>&1 || echo "HINTS_FAILED edge $h"
  HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/edge_$h.lzb timeout 3600 $R/host_${T}m agg-exec-fake > $O/cost/edge_$h.log 2>&1; rc=$?
  awk -v n=$h -v rc=$rc '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "edge %-7s chunks %12d aggregate %11d whole %12d %s (exit %s)\n", n, c, a, c+a, (bad?"INVALID":(a>0?"ok":"NO RESULT")), rc}' $O/cost/edge_$h.log
done
export RISC0_DEV_MODE=1 HAZYNC_CHUNKS=1
step "signet 1..$SN from genesis (dev mode) vs coinlog_ref.py, block_forest_ref.py"
G=$O/s$SN; rm -rf $G; $R/host_${T}sig libzync-prove-range 1 $SN $HOME/libzync-gpu/signet/json genesis $G > $G.log 2>&1; echo PR_EXIT=$?; grep -a 'LZ_PROVE_RANGE_DONE' $G.log; echo "PANICS=$(grep -ac panicked $G.log)"
STATES_OUT=${G}_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/signet/json $SN $G/state_$SN.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_2600.txt ${G}_states.jsonl $G/state_$SN.json | tail -1
echo RETEST_DONE
