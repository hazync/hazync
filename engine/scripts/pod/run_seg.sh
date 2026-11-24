#!/bin/bash
# On the pod: does the segment cap's padding decide proving time? (1) padded trace cells per cap for every leaf of
# EXPORT, by metered execution on the CUDA build's memory model (HZ_SEGMENTS; no GPU work), (2) each leaf proved alone
# at the caps in PROVE_CAPS. Results: out/seg_sweep.tsv, out/seg_prove.tsv.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; H=$PWD/host/target/release/hzengine; X=export/${EXPORT:-976379t}
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
CAPS=${CAPS:-"6 7 8 9 10 11 12 13 14 16 20 24"}
for f in $X/chunk_*.in $X/trie.in; do for g in $CAPS; do echo "$f $g"; done; done | \
  xargs -P ${SWEEP_PAR:-8} -n 2 sh -c 'r=$(HZ_SEGMENTS=1 HZ_SEG_MEM_GB=$1 '"$H"' execute --dir keys --input $0 2>&1 | grep -a "^segments"); printf "%s\t%s\t%s\n" "$(basename $0 .in)" "$1" "$r"' \
  | sort -k1,1 -k2,2n > out/seg_sweep.tsv
echo "sweep done"
# per job: the caps in 6..14 with the fewest and the most padded main cells, and 11 (the 3-provers default on 48 GB)
caps_for(){ awk -v j="$1" '$1==j && $2>=6 && $2<=14 {for(i=1;i<=NF;i++) if($i=="padded"){print $2, $(i+1); break}}' out/seg_sweep.tsv \
  | sort -k2,2n | awk 'NR==1{lo=$1} {hi=$1} END{print lo, hi, 11}' | tr " " "\n" | sort -nu | tr "\n" " "; }
for j in ${PROVE_JOBS:-chunk_00 trie}; do for g in ${PROVE_CAPS:-$(caps_for $j)}; do
  s=$(date +%s.%N)
  HZ_SEG_MEM_GB=$g $H prove --dir keys --input $X/$j.in --out out/p_${j}_$g > out/p_${j}_$g.log 2>&1; rc=$?
  e=$(echo "$(date +%s.%N) - $s" | bc)
  a=$(sed "s/\x1b\[[0-9;]*m//g" out/p_${j}_$g.log | grep -a "^INFO     app_prove" | grep -o "\[ [0-9.]*s" | tr -d "[ s")
  n=$(sed "s/\x1b\[[0-9;]*m//g" out/p_${j}_$g.log | grep -ac "prove_segment \[")
  printf "%s\t%s\trc=%s\twall=%.1f\tapp_prove=%s\tsegments=%s\n" "$j" "$g" "$rc" "$e" "$a" "$n" >> out/seg_prove.tsv
  rm -f out/p_${j}_$g.proof.json
done; done
# the aggregate: one prove-block (defaults) for its child proofs, then the same sweep and timing (it runs alone: caps 12-24)
$H prove-block --dir keys --export $X --out out/pb > out/pb.log 2>&1; echo "prove-block rc=$?"
kids=""; for c in $X/chunk_*.in; do kids="$kids --child out/pb/$(basename $c .in)"; done; kids="$kids --child out/pb/trie"
for g in 12 14 16 18 20 22 24; do
  r=$(HZ_SEGMENTS=1 HZ_SEG_MEM_GB=$g $H execute --dir keys --input $X/agg.in --self-id $kids 2>&1 | grep -a "^segments")
  printf "agg\t%s\t%s\n" "$g" "$r" >> out/seg_sweep.tsv; done
alo=$(awk '$1=="agg" {for(i=1;i<=NF;i++) if($i=="padded"){print $2, $(i+1); break}}' out/seg_sweep.tsv | sort -k2,2n | awk 'NR==1{lo=$1} {hi=$1} END{print lo, hi, 24}' | tr " " "\n" | sort -nu | tr "\n" " ")
for g in ${AGG_CAPS:-$alo}; do
  s=$(date +%s.%N)
  HZ_SEG_MEM_GB=$g $H prove --dir keys --input $X/agg.in --self-id $kids --out out/p_agg_$g > out/p_agg_$g.log 2>&1; rc=$?
  e=$(echo "$(date +%s.%N) - $s" | bc)
  a=$(sed "s/\x1b\[[0-9;]*m//g" out/p_agg_$g.log | grep -a "^INFO     app_prove" | grep -o "\[ [0-9.]*s" | tr -d "[ s")
  n=$(sed "s/\x1b\[[0-9;]*m//g" out/p_agg_$g.log | grep -ac "prove_segment \[")
  printf "agg\t%s\trc=%s\twall=%.1f\tapp_prove=%s\tsegments=%s\n" "$g" "$rc" "$e" "$a" "$n" >> out/seg_prove.tsv
done
rm -f out/pb/*.proof.json out/p_agg_*.proof.json
echo "rc=0" > out/RUN_DONE
