#!/bin/bash
# Prep's crash window between the trie's flush and its marker (review 11-12, #3), on disk stores, mainnet 1..60, a save
# point every 20 blocks, through the deployed restart script (coordinator/deploy/libzync/libzync-prep-run.sh):
#   1. the run dies right after the trie's flush at 40 (HAZYNC_TEST_PREP_CRASH_AT, dev mode): state_40.json, .smt and
#      .coinlog are there, state_40.trie is not, and the trie's own meta says 40;
#   2. prep from the OLDER save point (20, which the script used to take) is refused by the trie BEFORE the coin log
#      is touched: every coin-log file, the journal included, is byte-identical afterwards;
#   3. the script restarts from 40 (the trie's height), and 41..60 are byte-identical to an uninterrupted run.
#   4. stores but no save point (review 11-12, #8): the script clears them and redoes 1..60 from genesis.
#   --control: the script without the trie-height rule must take 20 and fail.
set -u
H=${H:-$HOME/dev/hazync-libzync/prover/target/fasthost/host}; B=${J:-$HOME/libzync-gpu/ssweep}; O=${O:-$HOME/libzync-gpu/crash}
HERE=$(cd "$(dirname "$0")" && pwd); RUN=${RUN:-$HERE/../../../coordinator/deploy/libzync/libzync-prep-run.sh}
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
rm -rf $O; mkdir -p $O
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_PREP_SAVE_EVERY=20
if [ $CONTROL = 1 ]; then
    sed 's/ || \[ "\$trie_h" = "\$h" \]//' "$RUN" > $O/run.sh
    grep -q 'trie_h" = "\$h"' $O/run.sh && { echo "CONTROL BROKEN: the rule is still in the copy"; exit 1; }
    RUN=$O/run.sh; echo "CONTROL: the restart script without the trie-height rule"
fi
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }

# the reference: one uninterrupted run
HAZYNC_COINLOG_DIR=$O/ref/cl HAZYNC_TRIE_DIR=$O/ref/trie $H libzync-prep 1 60 $B genesis $O/ref/out > $O/ref.log 2>&1
check "$([ $? = 0 ] && echo 1)" "reference run 1..60"

# the deployed layout: LZ_ROOT with blocks/ (consumed), cl/, trie/, out/, leaves/, bin/host_lz
R=$O/r; mkdir -p $R/blocks $R/bin
for h in $(seq 1 60); do cp $B/block_$h.json $R/blocks/; done
ln -s "$H" $R/bin/host_lz
LZ_ROOT=$R LZ_HI=60 HAZYNC_TEST_PREP_CRASH_AT=40 bash "$RUN" > $O/crash.log 2>&1; rc=$?
meta_h=$(python3 -c 'import sys,struct;b=open(sys.argv[1],"rb").read();print(struct.unpack_from("<Q",b,17)[0])' $R/trie/meta 2>/dev/null)
check "$([ $rc = 137 ] && [ -s $R/out/state_40.json ] && [ -e $R/out/state_40.coinlog ] && [ ! -e $R/out/state_40.trie ] && [ "$meta_h" = 40 ] && echo 1)" \
      "1. died after the trie flush at 40: no state_40.trie, the trie's meta at $meta_h (exit $rc)"

# 2. the old save point is refused before the coin log changes
before=$(cd $R/cl && sha256sum * | sha256sum)
HAZYNC_COINLOG_DIR=$R/cl HAZYNC_TRIE_DIR=$R/trie HAZYNC_PREP_CONTINUE=1 $H libzync-prep 21 60 $R/blocks $R/out/state_20.json $O/wrong > $O/wrong.log 2>&1; rc=$?
after=$(cd $R/cl && sha256sum * | sha256sum)
check "$([ $rc != 0 ] && grep -aq "disk trie is at height 40" $O/wrong.log && echo 1)" "2. prep from state_20 refused by the trie (exit $rc)"
check "$([ "$before" = "$after" ] && echo 1)" "2. ... and the coin log and its journal are untouched"

# 3. the restart script takes 40 and finishes as the uninterrupted run did
LZ_ROOT=$R LZ_HI=60 bash "$RUN" > $O/restart.log 2>&1; rc=$?
check "$([ $rc = 0 ] && grep -aq "libzync-prep: 41..60 from state_40.json" $O/restart.log && echo 1)" "3. the script restarts at 41 from state_40.json (exit $rc)"
same=1; for h in 41 50 59 60; do cmp -s $O/ref/out/state_$h.json $R/out/state_$h.json || same=0; done
check "$same" "3. states 41, 50, 59, 60 byte-identical to the uninterrupted run"

# 4. stores but no save point at all (a genesis run that died before its first save, review 11-12 #8): the script clears
#    them and runs from genesis, instead of restart-looping on "a coin-log store is already there"
rm -f $R/out/state_*; for h in $(seq 1 60); do cp $B/block_$h.json $R/blocks/; done
LZ_ROOT=$R LZ_HI=60 bash "$RUN" > $O/nosave.log 2>&1; rc=$?
check "$([ $rc = 0 ] && grep -aq "stores left by a run that died" $O/nosave.log && cmp -s $O/ref/out/state_60.json $R/out/state_60.json && echo 1)" \
      "4. no save point: the stores are cleared and 1..60 is redone from genesis (exit $rc)"

if [ $CONTROL = 1 ]; then
    [ $fails -ge 2 ] && { echo "CONTROL OK: $fails checks fail without the rule, as they must"; exit 0; }
    echo "CONTROL BROKEN: only $fails failed"; exit 1
fi
[ $fails = 0 ] && echo CRASH_TEST_OK || echo "CRASH_TEST_FAILED ($fails)"
