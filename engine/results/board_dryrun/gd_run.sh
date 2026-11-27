#!/bin/bash
# Gate 3 board dry run, part 2: the REAL coordinator (coordinator/server.py at the checkout) on 127.0.0.1:8911 with its
# own DB and dirs under /root/gd/coord, configured for the engine by environment only; one worker (coordinator/hazync)
# on this box's CPU: `hazync run` per block 1..8, then `hazync fold` and `hazync spine`; check-proofs on the stored
# ranges. Needs gd_setup.sh first. Usage: gd_run.sh
source ~/.cargo/env
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
R=/root/gd; S=$R/src; W=$R/run; K=/root/engine/keys-ch; D=$R/coord; P=8911
E=$S/engine/host/target/release/hzengine; H=$S/prover/target/release/host
PIN=$(awk '/^key_pin/{print $2}' $W/pack.out)
t(){ local s=$(date +%s); "$@"; local rc=$?; echo "STEP rc=$rc secs=$(( $(date +%s)-s )) :: $*"; return $rc; }
rm -rf $D && mkdir -p $D/state $D/proofs $D/spine $D/witnesses $R/worker
# the coordinator: the live libzync unit's settings (deploy/libzync/hazync-lz-coordinator.service), its own port and
# dirs, plus the engine's verify env (3.2: HAZYNC_ENGINE_VK + _PIN make `host verify-any` / `method-id` the engine's)
export HAZYNC_LIBZYNC=1 COORD_PORT=$P COORD_BIND=127.0.0.1 COORD_DB=$D/coordinator.db COORD_STATE=$D/state \
  COORD_PROOFS=$D/proofs COORD_SPINE=$D/spine WITNESS_DIR=$D/witnesses HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_HOST=$H \
  GENESIS_TIP=$($H libzync-genesis-tip) VERIFY_MODE=real SEED_RANGES=0 HAZYNC_LZ_STATE_DIR=$W/prep \
  HAZYNC_LZ_REORG_ACK_DIR=$D/reorg-acks HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN TIP_HEIGHT=8 \
  TIP_FILE=$D/node_tip
echo "GENESIS_TIP=$GENESIS_TIP"
(cd $S/coordinator && python3 server.py > $D/server.log 2>&1) & SRV=$!
for i in $(seq 60); do curl -sf http://127.0.0.1:$P/api/meta > $D/meta.json && break; sleep 1; done
echo "== /api/meta"; cat $D/meta.json; echo
# the worker: its own identity and dirs; the PROVER env (3.2: HAZYNC_ENGINE + _KEYS make the host prove on the engine)
for v in HAZYNC_LIBZYNC COORD_PORT COORD_BIND COORD_DB COORD_STATE COORD_PROOFS COORD_SPINE WITNESS_DIR HAZYNC_BRIDGE_OUT HAZYNC_HOST \
  GENESIS_TIP VERIFY_MODE SEED_RANGES HAZYNC_LZ_STATE_DIR HAZYNC_LZ_REORG_ACK_DIR HAZYNC_ENGINE_VK HAZYNC_ENGINE_VK_PIN TIP_HEIGHT TIP_FILE; do unset $v; done
export COORD_URL=http://127.0.0.1:$P HAZYNC_HOME=$R/worker HAZYNC_HOST=$H HAZYNC_ENGINE=$E HAZYNC_ENGINE_KEYS=$K RAYON_NUM_THREADS=8
HZ="python3 $S/coordinator/hazync"
$HZ id
echo "== leaves: hazync run, one block each"
for n in 1 2 3 4 5 6 7 8; do t nice $HZ run > $R/run_$n.log 2>&1; tail -3 $R/run_$n.log; done
echo "== state after the leaves"; curl -s http://127.0.0.1:$P/api/meta; echo
echo "== folds"; t nice $HZ fold 8 > $R/fold.log 2>&1; tail -8 $R/fold.log
echo "== spine"; t nice $HZ spine > $R/spine.log 2>&1; tail -8 $R/spine.log
curl -s http://127.0.0.1:$P/api/spine | head -c 600; echo; curl -s http://127.0.0.1:$P/api/meta; echo
echo "== check-proofs"
(cd $S/coordinator && t env COORD_DB=$D/coordinator.db COORD_PROOFS=$D/proofs HAZYNC_HOST=$H HAZYNC_LIBZYNC=1 HAZYNC_BRIDGE_OUT=$W/bundles \
   HAZYNC_LZ_STATE_DIR=$W/prep HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN CHECK_PROOFS_MIN_AGE=0 python3 check-proofs.py) 2>&1 | tail -8
echo "server pid $SRV (left running for the refusal step)"; echo $SRV > $D/server.pid
echo GD_RUN_DONE
