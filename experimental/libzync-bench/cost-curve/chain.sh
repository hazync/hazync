#!/bin/bash
# chain.sh: the runs after run_a.sh, one at a time on the shared box (what was run, in order, on hazync-coord).
# Paths are /root/cost-curve's layout: bin/ (the release host), inputs/ (leaf bundles + prep states), blocks/.
set -uo pipefail
R=/root/cost-curve; S=$R/src/cost-curve; HOST=$R/bin/hazync-host-x86_64-linux-gnu
cd $R
until grep -q RUN_A_DONE runA.out 2>/dev/null; do sleep 30; done
# (B) the curve, every 50k + 962,000 + a near-tip block (100k..400k overlap with A: the modelled-vs-real cross-check)
$S/run_b.sh $HOST blocks runB > runB.out 2>&1
# (B) dispersion: five consecutive blocks after 800,000 (how well one sample stands for its era)
$S/run_b.sh $HOST blocks runB 800001 800002 800003 800004 800005 >> runB.out 2>&1
# setup check: block 962,000 as ../profiles/run.sh runs it (the repo fixture, ONE chunk, po2 20) must give its
# published chunk 352,092,950 / trie 47,599,783 / aggregate 66,434,430 cycles
mkdir -p repro && ( cd repro && export HAZYNC_BLOCK=$R/blocks/fixture_962000.json HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost \
  HAZYNC_COINLOG_TXCOUNTS=$R/src/cost/txcounts.txt
  nice -n 10 $HOST libzync-hints c1.lzb > hints.log 2>&1
  HAZYNC_LZ_HINTS=$R/repro/c1.lzb nice -n 10 $HOST agg-exec-fake > agg.log 2>&1 )
# profiles: two real-state leaves, two modelled modern blocks
$S/run_prof.sh $HOST inputs blocks prof 200000 400000 -- 700000 976000 > prof.out 2>&1
echo CHAIN_DONE
