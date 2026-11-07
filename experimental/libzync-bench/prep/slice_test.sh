#!/bin/bash
# The slice-run controls (11-07): 4 fetch jobs over ssh RPC into consume-mode prep (save every 20, window 50) for
# mainnet 1..240, disk stores, PREP_PROGRESS every 40, leaf bundles only at 170,200,230, the check against the proved
# 1..200 only (past it: no reference, so no check), then the state at 240 vs an in-memory prep from the same blocks.
set -u
H=~/libzync-gpu/r18/host_c14bm; O=~/libzync-gpu/c14b/slice; rm -rf $O; mkdir -p $O; REF=$HOME/libzync-gpu/c14/m200
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost
F=~/dev/hazync-libzync/prover/fetch_block_rpc.py
HAZYNC_RPC_CLI="ssh hazync-coord bitcoin-cli" python3 $F range 1 240 $O/blocks --window 50 --jobs 4 > $O/fetch.log 2>&1 &
FP=$!
HAZYNC_COINLOG_DIR=$O/cl HAZYNC_TRIE_DIR=$O/trie HAZYNC_PREP_SAVE_EVERY=20 HAZYNC_PREP_CONSUME=1 HAZYNC_PREP_REPORT_EVERY=40 \
  HAZYNC_PREP_LEAF_BUNDLES=$O/leaves HAZYNC_PREP_LEAF_AT=170,200,230 HAZYNC_PREP_CHECK=$REF \
  $H libzync-prep 1 240 $O/blocks genesis $O/out > $O/prep.log 2>&1
echo "prep EXIT=$?"; grep -a 'PREP_PROGRESS\|LZ_PREP_DONE\|PREP_MISMATCH\|panicked' $O/prep.log | head -10
wait $FP; echo "fetch EXIT=$? ($(grep -c '^block .*prevouts' $O/fetch.log) blocks fetched), files left $(ls $O/blocks | wc -l)"
echo "leaf bundles: $(ls $O/leaves | tr '\n' ' ')"
cat $O/cl/SKIP; ls $O/cl
# the same blocks in memory (fetched again into a plain dir), for the state at 240
HAZYNC_RPC_CLI="ssh hazync-coord bitcoin-cli" python3 $F range 201 240 $O/b2 --jobs 4 > /dev/null 2>&1
cp ~/libzync-gpu/ssweep/block_{1..200}.json $O/b2/ 2>/dev/null
$H libzync-prep 1 240 $O/b2 genesis $O/mem > $O/mem.log 2>&1; echo "memory prep EXIT=$?"
cmp $O/out/state_240.json $O/mem/state_240.json && echo "disk+consume+4 jobs state_240 == in-memory prep's"
echo SLICE_TEST_DONE
