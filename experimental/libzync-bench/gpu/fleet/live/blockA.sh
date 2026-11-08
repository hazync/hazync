#!/bin/bash
# blockA.sh <H>: on pod A. Hints for block H, copy them to the peers, start the early aggregate and A's own lane.
H=$1; . /workspace/lzenv.sh; T=/workspace/tip_$H; rm -rf $T /workspace/q_$H /workspace/rc_$H; mkdir -p $T; mv /workspace/in_$H.json $T/block.json
K="-i /root/.ssh/eph -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
export HAZYNC_BLOCK=$T/block.json HAZYNC_CHUNKS=16
cd /workspace/hazync/prover
echo "HINTS_START $(date +%s.%N)" >> $T/times
target/release/host libzync-hints $T > $T/hints.log 2>&1 || { echo HINTS_FAIL >> $T/times; exit 1; }
echo "HINTS_END $(date +%s.%N)" >> $T/times
mkdir -p /workspace/q_$H /workspace/rc_$H
while read ip p; do ( ssh $K -p $p root@$ip "rm -rf $T; mkdir -p $T" && scp -q -r $K -P $p $T/block.json $T/ecmh.bin $T/hg root@$ip:$T/ ) & done < /workspace/peers.txt; wait
echo "DIST_END $(date +%s.%N)" >> $T/times
( HAZYNC_ECMH_HINTS=$T/ecmh.bin HAZYNC_BATCH_HINTS_DIR=$T/hg HAZYNC_AGG=1 HAZYNC_AGG_EARLY=1 HAZYNC_SEG_REMOTE=1 \
  HAZYNC_NO_PEERS_TIMEOUT=900 HAZYNC_RECEIPTS=/workspace/rc_$H HAZYNC_OUT=$T/block.receipt \
  setsid nohup bash -c "target/release/host seg-serve > $T/serve.log 2>&1; rc=\$?; echo \"SERVE_END \$(date +%s.%N) rc=\$rc\" >> $T/times" < /dev/null > /dev/null 2>&1 & )
(setsid nohup bash /workspace/lanew.sh $H self 127.0.0.1:9110 A < /dev/null > /dev/null 2>&1 &)
echo "BLOCKA_READY $(date +%s.%N)"
