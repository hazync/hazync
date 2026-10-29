#!/bin/bash
cd ~/libzync-gpu/neg; B=~/dev/hazync-libzync/prover/target/fasthost/host
export HAZYNC_BLOCK=$HOME/dev/hazync-libzync/prover/block_962000.json HAZYNC_LIFTX_HINT=1 HAZYNC_PROFILE_LAYOUT=cost HAZYNC_BATCH_ECDSA=1 HAZYNC_SWIFTSYNC=1 HAZYNC_AGG_READSLICE=1 HAZYNC_AGG_LITE=1 HAZYNC_CHUNKS=16
export $(sed 's/^FIT=//' ~/libzync-gpu/run/gb/libzync-bundle/fit_env.txt)
export HAZYNC_COST_MODEL=fit HAZYNC_PACK_KEYS=1 HAZYNC_COST_EC_OP=14537 HAZYNC_COST_EC_OP_REPEAT=0
for s in base t1_bitflip t2_negR t3_forge t4_truncate t5_allzero t6_schnorr_negR; do
  HAZYNC_BATCH_HINTS_DIR=$PWD/sc/$s $B exec-chunk 0 > out_$s.log 2>&1; echo "REAL_EXIT=$?" >> out_$s.log
  echo "$s: $(grep -o 'fallback=[0-9]*\|ok=[a-z]*' out_$s.log | tr '\n' ' ') $(grep -o 'all_valid=[0-9]*' out_$s.log) $(grep -o 'journal sha256 [0-9a-f]\{16\}' out_$s.log) $(tail -1 out_$s.log)"
done
echo NEG_DONE
