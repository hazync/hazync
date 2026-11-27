#!/bin/bash
# Gate 3 board dry run, part 3: refusals through the REAL submit path. The main board answers "already proven" (409)
# before it verifies anything, so these go to a SECOND, empty coordinator with the same engine configuration (port 8912,
# its own dirs), where every submission is verified: a control (block 1's real proof from part 2: accepted), a tampered
# journal byte, a tampered proof byte, another image's proof (the header zAPP's 0..4,031, image 163d229d) as range 1,
# and block 2's proof claimed as range 1. Usage: gd_refuse.sh
source ~/.cargo/env
for v in $(compgen -e | grep "^HAZYNC_"); do unset "$v"; done
R=/root/gd; S=$R/src; W=$R/run; D=$R/coord2; P=8912; H=$S/prover/target/release/host
PIN=$(awk '/^key_pin/{print $2}' $W/pack.out)
rm -rf $D && mkdir -p $D/state $D/proofs $D/spine $D/witnesses
( export HAZYNC_LIBZYNC=1 COORD_PORT=$P COORD_BIND=127.0.0.1 COORD_DB=$D/coordinator.db COORD_STATE=$D/state \
  COORD_PROOFS=$D/proofs COORD_SPINE=$D/spine WITNESS_DIR=$D/witnesses HAZYNC_BRIDGE_OUT=$W/bundles HAZYNC_HOST=$H \
  GENESIS_TIP=$($H libzync-genesis-tip) VERIFY_MODE=real SEED_RANGES=0 HAZYNC_LZ_STATE_DIR=$W/prep \
  HAZYNC_LZ_REORG_ACK_DIR=$D/reorg-acks HAZYNC_ENGINE_VK=$W/key.bin HAZYNC_ENGINE_VK_PIN=$PIN TIP_HEIGHT=8 TIP_FILE=$D/node_tip
  cd $S/coordinator && exec python3 server.py > $D/server.log 2>&1 ) & SRV=$!
for i in $(seq 60); do curl -sf http://127.0.0.1:$P/api/meta >/dev/null && break; sleep 1; done
G=$R/worker/receipts
sub() { # <label> <range> <file>: submit <file> as <range> from a fresh worker home (same key)
  local h=$D/w_$RANDOM; mkdir -p $h/receipts; cp $R/worker/key.hex $h/
  cp "$3" $h/receipts/$2.bin
  local s=$(date +%s%N); out=$(COORD_URL=http://127.0.0.1:$P HAZYNC_HOME=$h HAZYNC_HOST=$H HAZYNC_ENGINE=$S/engine/host/target/release/hzengine HAZYNC_ENGINE_KEYS=/root/engine/keys-ch python3 $S/coordinator/hazync submit $2 2>&1); rc=$?
  echo "== $1 (range $2): rc=$rc in $(( ($(date +%s%N)-s)/1000000 )) ms"; echo "$out" | grep -v '^\s*$' | tail -3
}
python3 -c "b=bytearray(open('$G/1.bin','rb').read());b[-5]^=1;open('$D/bad_journal.bin','wb').write(b)"
python3 -c "b=bytearray(open('$G/1.bin','rb').read());b[2000]^=1;open('$D/bad_proof.bin','wb').write(b)"
sub "tampered journal byte" 1 $D/bad_journal.bin
sub "tampered proof byte" 1 $D/bad_proof.bin
sub "another image's proof (headers 0..4,031)" 1 $S/zapps/headers/tests/data/engine4031/proof.hze
sub "block 2's proof as range 1" 1 $G/2.bin
sub "control: block 1's real proof" 1 $G/1.bin
curl -s http://127.0.0.1:$P/api/meta; echo
kill $SRV; echo GD_REFUSE_DONE
