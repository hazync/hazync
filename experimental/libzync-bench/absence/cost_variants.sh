#!/bin/bash
# E1 decision data: whole-block cycles (16 chunks + aggregate, execute mode, no GPU) for the coin-log image with and
# without the outpoint tree (o), the script tree (s) and both (os), on the coin-log cost table's 10 blocks. The trees
# are modelled at HAZYNC_ITREE_N live keys (default 180 M, about mainnet's UTXO set), as the coin log is (cl_synth).
# Each row also gives the PADDED cycles (every segment at its power of two: user + paging + padding), what proving pays.
# usage: cost_variants.sh <out dir> [images...]   default images: c10b c10o c10s c10os (~/libzync-gpu/r18/host_<img>)
R=~/libzync-gpu/r18; Z=~/dev/hazync-libzync; O=${1:?out dir}; shift
IMGS=${*:-c10b c10o c10s c10os}
mkdir -p $O
export HAZYNC_CHUNKS=16 HAZYNC_HINT_THREADS=2 HAZYNC_COINLOG_TXCOUNTS=$Z/experimental/libzync-bench/cost/txcounts.txt
unset RISC0_DEV_MODE
for b in $Z/prover/block_962000.json $HOME/libzync-gpu/sample/block_{970000,900000,850000,800000,750000,700000,650000}.json $R/exc/block_170060.json $R/exc/block_692261.json; do
  n=$(basename $b .json)
  for img in $IMGS; do
    HAZYNC_BLOCK=$b $R/host_$img libzync-hints $O/${n}_$img.lzb > /dev/null 2>&1
    HAZYNC_BLOCK=$b HAZYNC_LZ_HINTS=$O/${n}_$img.lzb $R/host_$img agg-exec-fake > $O/${n}_$img.log 2>&1
    awk -v n=$n -v i=$img '/chunk .*executed:/{c+=$(NF-1)} /chunk .* padded /{pc+=$4} /block validation cycles/{a=$NF} /block validation padded/{pa=$4} /REJECT|INVALID/{bad=1} END{printf "%-14s %-6s chunks %12d aggregate %11d whole %12d %s padded chunks %d aggregate %d whole %d\n", n, i, c, a, c+a, (bad?"INVALID":"ok"), pc, pa, pc+pa}' $O/${n}_$img.log
  done
done
echo COST_VARIANTS_DONE
