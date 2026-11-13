#!/bin/bash
# ROADMAP H-B e (11-07): libzync on the ORIGINAL coordinator + worker, dev mode, mainnet blocks 1..4.
# A local server.py (HAZYNC_LIBZYNC=1) serves prep's leaf bundles; the stock `hazync` worker proves leaves, submits
# them, proves + folds (joins) a range; the frontier must reach 4 from genesis. Negatives: verify-any with a wrong lo.
set -u
# H, O, J, REF, C overridable (CI)
H=${H:-~/libzync-gpu/r18/host_c14coord}; O=${O:-~/libzync-gpu/c14b/coord}; rm -rf $O; mkdir -p $O/bridge $O/w
J=${J:-$HOME/libzync-gpu/ssweep}; REF=${REF:-$HOME/libzync-gpu/c14/m200}; C=${C:-$(cd "$(dirname "$0")/../../../coordinator" && pwd)}
export RISC0_DEV_MODE=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
# dev-mode receipts are fakes: verify-any accepts them only with this second switch (and says fake=1), and the board
# takes a fake=1 line only under COORD_ALLOW_MOCK. Without the switches both refuse (checked at the end).
export HAZYNC_TEST_FAKE_RECEIPTS=1
HAZYNC_PREP_LEAF_BUNDLES=$O/bridge HAZYNC_PREP_CHECK=$REF $H libzync-prep 1 8 $J genesis $O/prep > $O/prep.log 2>&1
echo "prep EXIT=$? bundles $(ls $O/bridge | grep -c '^leaf_')"
GT=$($H libzync-genesis-tip); echo "genesis tip ${GT:0:16}"
PORT=18931
( cd $C && COORD_ALLOW_MOCK=1 COORD_PORT=$PORT COORD_BIND=127.0.0.1 COORD_DB=$O/c.db COORD_SPINE=$O/spine COORD_PROOFS=$O/proofs \
  COORD_WEB=$C HAZYNC_HOST=$H HAZYNC_LIBZYNC=1 HAZYNC_BRIDGE_OUT=$O/bridge GENESIS_TIP=$GT \
  exec python3 server.py > $O/server.log 2>&1 ) &
SP=$!
for i in $(seq 1 60); do curl -s http://127.0.0.1:$PORT/api/meta >/dev/null && break; sleep 1; done
curl -s http://127.0.0.1:$PORT/api/meta | head -c 300; echo
W="env HAZYNC_HOME=$O/w COORD_URL=http://127.0.0.1:$PORT HAZYNC_HOST=$H python3 $C/hazync"
$W id > $O/w_id.log 2>&1; echo "worker id EXIT=$?"
for h in 1 2; do
  $W prove $h > $O/prove$h.log 2>&1; echo "prove $h EXIT=$?"; grep -a 'archive-node\|receipt:\|Error\|error' $O/prove$h.log | head -3
  $W submit $h > $O/submit$h.log 2>&1; echo "submit $h EXIT=$?"; tail -2 $O/submit$h.log
done
# a range: leaves 3 and 4 proved and submitted as it goes, then folded (a libzync join) into 3-4
$W run 3-4 > $O/run34.log 2>&1; echo "run 3-4 EXIT=$?"; grep -a 'archive-node\|folding\|✓\|✗\|rror' $O/run34.log | head -6
# the coordinator's verify of a receipt against a WRONG start height must refuse
HAZYNC_BRIDGE_OUT=$O/bridge $H verify-any $O/w/receipts/2.bin 2 | grep -a RANGE-OK | cut -c1-60
HAZYNC_BRIDGE_OUT=$O/bridge $H verify-any $O/w/receipts/2.bin 1 > $O/neg.log 2>&1; echo "verify-any block 2 claimed as starting at 1 EXIT=$? (want 1): $(grep -a REJECT $O/neg.log | head -c 90)"
HAZYNC_BRIDGE_OUT=$O/bridge $H verify-any $O/w/receipts/3-4.bin 4 > $O/neg2.log 2>&1; echo "verify-any 3-4 claimed as starting at 4 EXIT=$? (want 1): $(grep -a REJECT $O/neg2.log | head -c 90)"
# without the test switch, a fake receipt is not a proof
env -u HAZYNC_TEST_FAKE_RECEIPTS HAZYNC_BRIDGE_OUT=$O/bridge $H verify-any $O/w/receipts/2.bin 2 > $O/neg3.log 2>&1; echo "verify-any fake without the test switch EXIT=$? (want 1): $(grep -a REJECT $O/neg3.log | head -c 90)"
curl -s http://127.0.0.1:$PORT/api/state > $O/state.json
python3 - $O/state.json <<'EOF'
import json, sys
s = json.load(open(sys.argv[1]))
keys = {k: s[k] for k in s if any(w in k for w in ("frontier", "proven", "anchored"))}
print("board:", json.dumps(keys)[:400])
EOF
kill $SP; wait $SP 2>/dev/null
grep -a -i 'traceback\|error' $O/server.log | head -5
echo COORD_TEST_DONE
