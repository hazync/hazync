#!/bin/bash
# Prep CONTINUING under another image (11-08): a run saved at 100 under a different self_id (the laptop build's
# bbf82800, written into state_100.json) carried on 101..200 by the published-image host (863c9ce3) must give the
# same states and leaf bundles as one uninterrupted 1..200 run by that host.
set -u
H=${H:-$HOME/dev/hazync-libzync/prover/target/fasthost/host}; B=${J:-$HOME/libzync-gpu/ssweep}; O=${O:-$HOME/libzync-gpu/restamp}
rm -rf $O; mkdir -p $O
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_PREP_SAVE_EVERY=20 HAZYNC_PREP_LEAF_AT=101,150,200
run() { HAZYNC_COINLOG_DIR=$O/$1/cl HAZYNC_TRIE_DIR=$O/$1/trie HAZYNC_PREP_LEAF_BUNDLES=$O/$1/leaves "${@:2}"; }
mkdir -p $O/a/leaves $O/b/leaves
run a $H libzync-prep 1 200 $B genesis $O/a/out > $O/a.log 2>&1; echo "A EXIT=$?"
run b $H libzync-prep 1 100 $B genesis $O/b/out > $O/b1.log 2>&1; echo "B1 EXIT=$?"
python3 - "$O/b/out/state_100.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d["self_id"] = [2685115, 1073633663, 252104871, 2198851930, 406298722, 1022103293, 3024406779, 3979199044]  # bbf82800
json.dump(d, open(p, "w"))
PY
run b env HAZYNC_PREP_CONTINUE=1 $H libzync-prep 101 200 $B $O/b/out/state_100.json $O/b/out > $O/b2.log 2>&1; echo "B2 EXIT=$?"
grep -a "re-stamped" $O/b2.log
ok=1
for h in 101 150 199 200; do cmp -s $O/a/out/state_$h.json $O/b/out/state_$h.json && echo "state_$h identical" || { echo "state_$h DIFFERS"; ok=0; }; done
for h in 101 150 200; do cmp -s $O/a/leaves/leaf_$h.bin $O/b/leaves/leaf_$h.bin && echo "leaf_$h identical" || { echo "leaf_$h DIFFERS"; ok=0; }; done
[ $ok = 1 ] && echo RESTAMP_TEST_OK || echo RESTAMP_TEST_FAILED
