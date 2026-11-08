#!/bin/bash
# lanew.sh <H> <self|ip:sshport> <seg addr> <id>: work-stealing chunk lane for block H, then an aggregate worker.
# Chunks are claimed on pod A with an atomic mkdir; receipts land on A via write-to-.tmp + rename.
H=$1; A=$2; SEG=$3; ID=$4
. /workspace/lzenv.sh; T=/workspace/tip_$H; O=$T/lane; mkdir -p $O
export HAZYNC_BLOCK=$T/block.json HAZYNC_ECMH_HINTS=$T/ecmh.bin HAZYNC_CHUNKS=16 HAZYNC_BATCH_HINTS_DIR=$T/hg
K="-i /root/.ssh/eph -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
CLAIM='for i in $(seq 0 15); do mkdir /workspace/q_'$H'/c$i 2>/dev/null && { echo $i; exit; }; done; echo NONE'
onA(){ if [ "$A" = self ]; then bash -c "$1"; else ssh $K -p ${A#*:} root@${A%:*} "$1"; fi; }
cd /workspace/hazync/prover
echo "START $(date +%s.%N)" >> $O/lane.tsv
while true; do
  i=$(onA "$CLAIM"); [ "$i" = NONE ] || [ -z "$i" ] && break
  A0=$(date +%s.%N)
  for ATT in 1 2 3 4; do
    HAZYNC_OUT=$O/chunk_$i.hzk target/release/host prove-chunk $i > $O/prove_$i.log 2>&1; RC=$?
    [ $RC -eq 0 ] && break
    grep -q "verification indicates proof is invalid" $O/prove_$i.log || break
  done
  [ $RC -eq 0 ] || { echo "FAIL $i" >> $O/lane.tsv; exit 1; }
  if [ "$A" = self ]; then cp $O/chunk_$i.hzk /workspace/rc_$H/chunk_$i.hzk.tmp; mv /workspace/rc_$H/chunk_$i.hzk.tmp /workspace/rc_$H/chunk_$i.hzk
  else scp -q $K -P ${A#*:} $O/chunk_$i.hzk root@${A%:*}:/workspace/rc_$H/chunk_$i.hzk.tmp && onA "mv /workspace/rc_$H/chunk_$i.hzk.tmp /workspace/rc_$H/chunk_$i.hzk"; fi
  echo -e "$i\t$A0\t$(date +%s.%N)" >> $O/lane.tsv
done
echo "END $(date +%s.%N)" >> $O/lane.tsv
HAZYNC_WORKER_ID=$ID target/release/host seg-connect $SEG > $O/worker.log 2>&1
echo "WORKER_END $(date +%s.%N)" >> $O/lane.tsv
