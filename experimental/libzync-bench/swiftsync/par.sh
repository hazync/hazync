#!/bin/bash
# Design B verdict tests at H = 200, in parallel: correct hints must BALANCE; one flipped bit each way must NOT.
R=~/libzync-gpu/r15; S=~/libzync-gpu/ssweep; H=$R/host15
export HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_BATCH_ECDSA=1 HAZYNC_SWIFTSYNC=1 HAZYNC_AGG_READSLICE=1 HAZYNC_AGG_LITE=1 HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2
$H swiftsync-make-hints 200 $S $R/h200.hints 2>&1 | tail -1
for t in ok flip_unspent_to_spent flip_spent_to_unspent; do
  case $t in ok) F="";; flip_unspent_to_spent) F="HAZYNC_SS_FLIP=1:0";; flip_spent_to_unspent) F="HAZYNC_SS_FLIP=9:0";; esac
  ( env $F HAZYNC_SS_HINTS=$R/h200.hints $H swiftsync-sweep 200 $S > $R/s200_$t.log 2>&1; echo "REAL_EXIT=$?" >> $R/s200_$t.log ) &
done
wait
for t in ok flip_unspent_to_spent flip_spent_to_unspent; do
  echo "$t: $(grep -E 'ss_set=|SWEEP_|REJECTED|panicked|NEGATIVE' $R/s200_$t.log | tr '\n' ' ') $(tail -1 $R/s200_$t.log)"
done
echo PAR_DONE
