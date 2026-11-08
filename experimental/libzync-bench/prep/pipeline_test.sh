#!/bin/bash
# Whole-chain prep pipeline in miniature: a producer fetches blocks over RPC (the bridge node, read-only) into a
# window; prep consumes them in disk mode, saving every 10; a crash at 35; a restart from the save at 30 with the
# block files below it deleted (the BIP30 tree from state_30.smt). Every state checked against the proved chain.
set -u
H=~/libzync-gpu/r18/host_c14bm; Z=~/dev/hazync-libzync; O=~/libzync-gpu/c14b/pipe; rm -rf $O; mkdir -p $O
REF=$HOME/libzync-gpu/c14/m200
export HAZYNC_RPC_CLI="ssh hazync-coord bitcoin-cli" HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1
export HAZYNC_COINLOG_DIR=$O/cl HAZYNC_TRIE_DIR=$O/trie HAZYNC_PREP_SAVE_EVERY=10 HAZYNC_PREP_CONSUME=1
python3 $Z/prover/fetch_block_rpc.py range 1 60 $O/blocks --window 15 > $O/producer.log 2>&1 & P=$!
HAZYNC_CRASH_AT=prep-after:35 HAZYNC_PREP_CHECK=$REF $H libzync-prep 1 60 $O/blocks genesis $O/out > $O/prep1.log 2>&1; echo "prep crash at 35 EXIT=$? (want 9)"
echo "block files left: $(ls $O/blocks | grep -c json) (lowest: $(ls $O/blocks | sed 's/block_//;s/.json//' | sort -n | head -1)); state_30.smt: $(ls $O/out/state_30.smt 2>/dev/null)"
HAZYNC_PREP_CONTINUE=1 HAZYNC_PREP_CHECK=$REF $H libzync-prep 31 60 $O/blocks $O/out/state_30.json $O/out2 > $O/prep2.log 2>&1; echo "restart from 30 EXIT=$?"
grep -a 'undid\|LZ_PREP_DONE\|PREP_MISMATCH\|panicked' $O/prep2.log | head -3
wait $P; echo "producer EXIT=$?"; echo "block files left at the end: $(ls $O/blocks | grep -c json)"
echo PIPELINE_TEST_DONE
