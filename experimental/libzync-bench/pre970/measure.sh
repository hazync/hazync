#!/bin/bash
# The last blocks before H = 970,000 (the published mainnet hints file's stop height): execute-mode cycles with the
# file's REAL hint bits vs with no hints file (mode 0, every output to ECMH, as at the tip).
P=~/libzync-gpu/pre970; H=~/libzync-gpu/r18/host_main; O=$P/exec; mkdir -p $O
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=4 HAZYNC_HINT_THREADS=2
for h in 969996 969997 969998 969999 970000; do
  export HAZYNC_BLOCK=$P/json/block_$h.json
  for m in hints nohints; do
    if [ $m = hints ]; then export HAZYNC_SS_HINTS=~/libzync-gpu/hints/bitcoin.hints; else unset HAZYNC_SS_HINTS; fi
    $H libzync-hints $O/b${h}_$m.lzb > $O/${h}_${m}_hints.log 2>&1
    HAZYNC_LZ_HINTS=$O/b${h}_$m.lzb $H agg-exec-fake > $O/${h}_${m}_agg.log 2>&1
    c=$(grep -a -oE 'executed: [0-9]+ cycles' $O/${h}_${m}_agg.log | awk '{s+=$2} END {print s}')
    a=$(grep -a -oE 'block validation cycles +[0-9]+' $O/${h}_${m}_agg.log | awk '{print $4}')
    ss=$(grep -a -o 'swiftsync: adds=[0-9]* subs=[0-9]*' $O/${h}_${m}_agg.log | tail -1)
    echo -e "$h\t$m\tchunks=$c\tagg=${a:-FAIL}\t$ss" | tee -a $P/measure.tsv
  done
done
echo MEASURE_DONE >> $P/measure.tsv
