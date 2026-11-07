#!/bin/bash
# Cost survey on the frozen core c14 (product image: coin log, counting trie as its own receipt, live-coin hash):
# execute-mode cycles per block = chunk + aggregate + trie update, for the 19 sample blocks (every 50k, 100,000 ..
# 970,000), one chunk per block as the round-16/17 survey. The coin log and trie around each block are MODELLED
# (lz::cl_synth_around: the block's own spends, real tx counts), so the trie update's cycles are a modelled trie's.
#   bash survey_c14.sh <host_c14m> > survey_c14.out
H=${1:?host binary of the c14 mainnet image}; S=~/libzync-gpu/sample; O=~/libzync-gpu/survey_c14; mkdir -p $O
export HAZYNC_PROFILE_LAYOUT=cost HAZYNC_CHUNKS=1 HAZYNC_HINT_THREADS=2 HAZYNC_COINLOG_TXCOUNTS=$(dirname "$0")/txcounts.txt
echo -e "build\theight\tchunk_cycles\tagg_cycles\ttotal\ttrie_cycles"
for f in $(ls $S/block_*.json | sort -t_ -k2 -n); do
  h=$(basename $f .json | cut -d_ -f2)
  HAZYNC_BLOCK=$f $H libzync-hints $O/$h.lzb > $O/$h.hints.log 2>&1 || { echo -e "on\t$h\tHINTS_FAILED"; continue; }
  HAZYNC_BLOCK=$f HAZYNC_LZ_HINTS=$O/$h.lzb $H agg-exec-fake > $O/$h.log 2>&1; rc=$?
  c=$(grep -a 'chunk .*executed:' $O/$h.log | grep -a -oE 'executed: [0-9]+' | awk '{s+=$2} END {print s+0}')
  a=$(grep -a -oE 'block validation cycles +[0-9]+' $O/$h.log | awk '{print $4}')
  t=$(grep -a -oE 'trie update executed: [0-9]+' $O/$h.log | awk '{print $4}')
  echo -e "on\t$h\t$c\t${a:-FAIL}\t$((c + ${a:-0} + ${t:-0}))\t${t:-0}\trc=$rc"
done
echo SURVEY_DONE
