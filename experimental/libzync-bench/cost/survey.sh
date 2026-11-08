#!/bin/bash
# Cost survey: execute-mode cycles (one chunk + aggregate) for 19 sample blocks across history, scripts ON
# (round-16 guest) and OFF (round-17 no-scripts guest), with real hint bits from the published mainnet hints file.
R=~/libzync-gpu/r17; S=~/libzync-gpu/sample
export HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_BATCH_ECDSA=1 HAZYNC_SWIFTSYNC=1 HAZYNC_AGG_READSLICE=1 HAZYNC_AGG_LITE=1
export HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=1 HAZYNC_SS_HINTS=$HOME/libzync-gpu/hints/bitcoin.hints
echo -e "build\theight\tchunk_cycles\tagg_cycles\ttotal" > $R/survey.tsv
for b in on off; do
  [ $b = on ] && H=~/libzync-gpu/r16/host16 || H=$R/host17_noscripts
  for f in $(ls $S/block_*.json | sort -t_ -k2 -n); do
    h=$(basename $f .json | cut -d_ -f2); O=$R/s_${b}_$h; mkdir -p $O
    HAZYNC_BLOCK=$f $H libzync-hints $O > $O/hints.log 2>&1
    HAZYNC_BLOCK=$f HAZYNC_BATCH_HINTS_DIR=$O/hg HAZYNC_ECMH_HINTS=$O/ecmh.bin $H agg-exec-fake > $O/agg.log 2>&1; rc=$?
    c=$(grep -a -oE 'executed: [0-9]+ cycles' $O/agg.log | awk '{s+=$2} END {print s+0}')
    a=$(grep -a -oE 'block validation cycles +[0-9]+' $O/agg.log | awk '{print $4}')
    echo -e "$b\t$h\t$c\t${a:-FAIL}\t$((c + ${a:-0}))\trc=$rc" | tee -a $R/survey.tsv
  done
done
echo SURVEY_DONE
