#!/bin/bash
# ROADMAP H4: the full test pass on the product image (c10e12: coin log + trie receipt + live-coin hash + join mode).
# Unit tests; rig vs Core 28.0 (regtest); handover O -> N; mainnet 1..200 vs the reference implementations; cost on
# real blocks (vs c10e11); edge blocks; signet 1..2600 from genesis. Each step prints its own verdict line.
R=~/libzync-gpu/r18; O=~/libzync-gpu/c12; Z=~/dev/hazync-libzync; CR=$Z/experimental/libzync-bench/core_rules; S=$Z/scripts/libzync
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
step "build regtest O (c12reg)"; build c12reg $B HAZYNC_NETWORK=regtest
step "rig vs Core 28.0"
rm -rf $O/rig; mkdir -p $O/rig
RIG_COINLOG=1 BITCOIND=$HOME/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind PYTHONUNBUFFERED=1 RIG_VERBOSE=1 python3 $CR/rig.py $R/host_c12reg $O/rig > $O/rig.out 2>&1; echo RIG_EXIT=$?
grep -a '^[A-Z-]\{4,\} ' $O/rig.out | grep -v '^AGREE'; tail -1 $O/rig.out
step "build regtest N: O pinned at 101 (pin not committed)"
P=$Z/prover/methods/guest/src/lineage/regtest_coinlog.rs; cp $P $O/lineage.rs.orig
W=$($R/host_c12reg method-id 2>/dev/null | sed -n 's/.*u32x8 *\[\(.*\)\]/\1/p')
sed -i "s/^pub const LINEAGE: &\[crate::handover_pin::Handover\] = &\[\];/pub const LINEAGE: \&[crate::handover_pin::Handover] = \&[crate::handover_pin::Handover { image: [$W], first_height: 101 }];/" $P
grep -q 'first_height: 101' $P || { echo PIN_EDIT_FAILED; cp $O/lineage.rs.orig $P; exit 1; }
bash $R/build.sh c12regN $B HAZYNC_NETWORK=regtest; cp $O/lineage.rs.orig $P
grep -a 'REAL_EXIT\|^METHOD_ID' $R/build_c12regN.log; [ -x $R/host_c12regN ] || { echo BUILD_FAILED N; exit 1; }
( cd $Z && git diff --quiet -- prover/methods/guest/src/lineage/regtest_coinlog.rs ) && echo "pin reverted" || echo "PIN NOT REVERTED"
step "handover O -> N"
rm -rf $O/ho; mkdir -p $O/ho
bash $Z/experimental/libzync-bench/handover/run.sh $R/host_c12reg $R/host_c12regN $O/rig/blocks/main $O/ho 2>&1 | grep -a 'PASS\|WRONG\|as expected'; echo HO_EXIT=${PIPESTATUS[0]}
step "build signet (c12sig)"; build c12sig $B HAZYNC_NETWORK=signet
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
step "mainnet 1..200 from genesis (c10e12, the store test's run) vs coinlog_ref.py, block_forest_ref.py, genesis anchor"
M=$HOME/libzync-gpu/e2s8/a
STATES_OUT=$O/m200_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/ssweep 200 $M/state_200.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_200.txt $O/m200_states.jsonl $M/state_200.json | tail -1
python3 $S/genesis_anchor_id.py mainnet $(printf '0%.0s' {1..64}) | tail -1; python3 -c "import json;s=json.load(open('$M/state_200.json'));print('state anchor_id', bytes(s['anchor_id']).hex(), 'version', s['version'], 'profile', hex(s['profile']))"
step "cost on real blocks: c10e12 vs c10e11, execute mode, 16 chunks"
unset RISC0_DEV_MODE; export HAZYNC_CHUNKS=16 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
mkdir -p $O/cost
for b in $Z/prover/block_962000.json $HOME/libzync-gpu/sample/block_{970000,900000,850000,800000,750000,700000,650000}.json $R/exc/block_170060.json $R/exc/block_692261.json; do
  n=$(basename $b .json)
  for img in c10e11 c10e12; do
    HAZYNC_BLOCK=$b $R/host_$img libzync-hints $O/cost/${n}_$img.lzb > /dev/null 2>&1
    HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/${n}_$img.lzb $R/host_$img agg-exec-fake > $O/cost/${n}_$img.log 2>&1
    awk -v n=$n -v i=$img '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "%-14s %-6s chunks %12d aggregate %11d whole %12d %s\n", n, i, c, a, c+a, (bad?"INVALID":"ok")}' $O/cost/${n}_$img.log
  done
done
step "edge blocks: BIP30 repeats, activation boundaries, first retarget, the largest legacy tx (c10e12, modelled log)"
for h in 2016 91842 91880 185956 227931 363725 364292 388381 419328 481824; do
  b=$HOME/libzync-gpu/edge/block_$h.json
  HAZYNC_BLOCK=$b $R/host_c10e12 libzync-hints $O/cost/edge_$h.lzb > /dev/null 2>&1
  HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/cost/edge_$h.lzb timeout 3600 $R/host_c10e12 agg-exec-fake > $O/cost/edge_$h.log 2>&1; rc=$?
  awk -v n=$h -v rc=$rc '/chunk .*executed:/{c+=$(NF-1)} /block validation cycles/{a=$NF} /REJECT|INVALID/{bad=1} END{printf "edge %-7s chunks %12d aggregate %11d whole %12d %s (exit %s)\n", n, c, a, c+a, (bad?"INVALID":(a>0?"ok":"NO RESULT")), rc}' $O/cost/edge_$h.log
done
export RISC0_DEV_MODE=1 HAZYNC_CHUNKS=1
step "signet 1..2600 from genesis (dev mode) vs coinlog_ref.py, block_forest_ref.py"
rm -rf $O/s2600; $R/host_c12sig libzync-prove-range 1 2600 $HOME/libzync-gpu/signet/json genesis $O/s2600 > $O/s2600.log 2>&1; echo PR_EXIT=$?; grep -a 'LZ_PROVE_RANGE_DONE\|panicked' $O/s2600.log | grep -v 'collect run' | tail -2
STATES_OUT=$O/s2600_states.jsonl python3 $S/coinlog_ref.py $HOME/libzync-gpu/signet/json 2600 $O/s2600/state_2600.json
python3 $S/block_forest_ref.py check $HOME/libzync-gpu/r17/hashes_2600.txt $O/s2600_states.jsonl $O/s2600/state_2600.json | tail -1
echo RETEST_DONE
