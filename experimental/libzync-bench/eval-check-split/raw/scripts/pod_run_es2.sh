#!/bin/bash
# pod_run_es2.sh: the CMP..DONE half of pod_run_es.sh, restarted by hand (see GOOD below). Original header:
# pod_run_es.sh (on the pod, /workspace): eval_check split re-profile, 23.11.2026. REAL proofs (no dev mode), one card.
# Adapted from ../../paper/d8/pod_run_d8.sh. Phases (out/phase), every step timed into out/steps.tsv:
#   SETUP   release v0.27.1 (CUDA host, CPU host for the guest ELF, verifier), sums checked against the laptop's
#           PGP-checked SHA256SUMS hash, method id 46666b6f; CUDA 12.8 toolkit (nvcc, nsys, ncu)
#   A       stock release host: warm 170, then 170, 300000, 200000 timed (nothing else running)
#   BUILD   perf/eval-check-split (git clone), provision-vps.sh GPU=1 with the canonical ELF, in the background;
#           meanwhile A-prof: nsys (300000) + ncu (170) + cuobjdump resource usage of the release host
#   CPUREAL a real eval_check input dumped by the branch host (leaf 170, HAZYNC_SEG_PO2=16), split/test_split_cpu.sh
#   CMP     HAZYNC_EVAL_CHECK=compare:<v> on 300000 for split, splitim, chainim: every call bit-identical
#   B       timing: branch host stock/split/splitim/chainim on 300000 and 200000, release 300000 again;
#           nsys per mode on 300000; ncu of the variant kernels on 170
#   C       fixed cost: method-id, nsys timeline of 170, mainnet 1..2 in one process vs two processes
#   VERIFY  every receipt with the release verifier
# B's variant timing is skipped (C and VERIFY still run) unless CPUREAL printed SPLIT_CPU_OK and CMP had no mismatch.
W=/workspace; O=$W/out; I=$W/in; mkdir -p $O/prof; cd $W || exit 1
unset RISC0_DEV_MODE HAZYNC_CHUNKS HAZYNC_PROFILE_LAYOUT HAZYNC_SEG_PO2 HAZYNC_BRIDGE_OUT HAZYNC_EVAL_CHECK
BRANCH=perf/eval-check-split
step() {  # step <label> <log> <cmd...>: timed, one retry (hazync#119)
  local lab=$1 log=$2; shift 2
  for try in 1 2; do
    local t0=$(date +%s.%N); "$@" >> "$log" 2>&1; local rc=$?; local t1=$(date +%s.%N)
    echo -e "$lab\t$try\t$rc\t$(python3 -c "print(round($t1-$t0,1))")\t$(date -u +%T)" >> $O/steps.tsv
    [ $rc = 0 ] && return 0
  done; return 1; }
once() {  # once <label> <log> <cmd...>: timed, no retry (profilers)
  local lab=$1 log=$2; shift 2
  local t0=$(date +%s.%N); "$@" >> "$log" 2>&1; local rc=$?; local t1=$(date +%s.%N)
  echo -e "$lab\t1\t$rc\t$(python3 -c "print(round($t1-$t0,1))")\t$(date -u +%T)" >> $O/steps.tsv; return $rc; }
get() {  # get <release> <dir> <asset...>
  local v=$1 d=$2; shift 2; mkdir -p $d
  for f in SHA256SUMS.txt "$@"; do for t in 1 2 3; do [ -s $d/$f ] && break
    curl -fsSL -o $d/$f.tmp https://github.com/hazync/hazync/releases/download/$v/$f && mv $d/$f.tmp $d/$f; done; done; }
leaf() {  # leaf <host> <h> <tag> [VAR=val...]: prove leaf h into $O/<tag>_<h>
  local H=$1 h=$2 tag=$3; shift 3; rm -rf $O/${tag}_$h
  ( cd $O && step "$tag leaf $h" $O/${tag}_$h.log env "$@" HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_$h.bin $H libzync-prove-range $h $h unused leaf ${tag}_$h ); }

NCUM=launch__registers_per_thread,launch__grid_size,launch__block_size,launch__occupancy_limit_registers,sm__warps_active.avg.pct_of_peak_sustained_active,gpu__time_duration.sum,l1tex__t_bytes_pipe_lsu_mem_local_op_ld.sum,l1tex__t_bytes_pipe_lsu_mem_local_op_st.sum,l1tex__t_bytes_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,dram__bytes_write.sum,smsp__thread_inst_executed.sum,launch__stack_size
ncu_run() {  # ncu_run <tag> <host> <kernel regex> <count> [VAR=val...]
  local tag=$1 H=$2 k=$3 c=$4; shift 4; [ -n "$NCU" ] || return 1
  ( cd $O && once "ncu $tag" $O/prof/ncu_$tag.log env "$@" HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_170.bin timeout 2400 $NCU --replay-mode application \
      --target-processes all -k "regex:$k" -c $c --metrics $NCUM --csv --log-file $O/prof/ncu_$tag.csv $H libzync-prove-range 170 170 unused leaf ncu_${tag}_170 ) ; }
H0=$W/rel/hazync-host-x86_64-linux-gnu-cuda; V=$W/rel/hazync-verify-x86_64-linux-gnu
NSYS=/opt/nvidia/nsight-systems/2024.6.2/bin/nsys; NCU=/opt/nvidia/nsight-compute/2024.1.1/ncu; CUOBJ=/usr/local/cuda-12.8/bin/cuobjdump
H1=$W/hz/prover/target/release/host
# the first build's binary failed cudaMemcpyToSymbol(poly_mix) (the --inline-mul TUs); rebuilt at 7a8079ff
while [ ! -s $O/REBUILD_RC ]; do sleep 10; done
[ "$(cat $O/REBUILD_RC)" = 0 ] || { echo "REBUILD_FAILED" > $O/phase; echo "OK" > $O/RUN_DONE; exit 1; }
$H1 method-id > $O/method_id_branch2.txt 2>&1
$CUOBJ --dump-resource-usage $H1 2>/dev/null | python3 -I $I/resusage.py > $O/prof/resusage_branch2.txt 2>&1
echo "CPUREAL" > $O/phase
mkdir -p $O/old1 && mv $O/cmp_* $O/dump_170* $O/cpu_test.log $O/old1/ 2>/dev/null
rm -rf $O/dump170 $W/dump170; mkdir -p $O/dump170
leaf $H1 170 dump HAZYNC_SEG_PO2=16 HAZYNC_EVAL_CHECK_DUMP=$O/dump170
[ -s $O/dump170/meta.txt ] || leaf $H1 170 dump21 HAZYNC_EVAL_CHECK_DUMP=$O/dump170 HAZYNC_EVAL_CHECK_DUMP_MAX_DOMAIN=8388608
( export PATH=$HOME/.cargo/bin:$PATH; cd $W/hz/vendor/risc0-circuit-rv32im-sys &&
  once "cpu test random" $O/cpu_test.log split/test_split_cpu.sh $W/cpub2 &&
  once "cpu test real" $O/cpu_test.log $W/cpub2/test_split_cpu dump $O/dump170 10 )
du -sh $O/dump170 >> $O/cpu_test.log; cp $O/dump170/meta.txt $O/dump170_meta.txt; mv $O/dump170 $W/dump170
echo "CMP" > $O/phase
for v in split; do leaf $H1 300000 cmp_$v HAZYNC_EVAL_CHECK=compare:$v; leaf $H1 200000 cmp_$v HAZYNC_EVAL_CHECK=compare:$v; done
calls=$(cat $O/cmp_*.log | /usr/bin/grep -c 'HAZYNC_EVAL_CHECK call=' || true)
bad=$(cat $O/cmp_*.log | /usr/bin/grep 'HAZYNC_EVAL_CHECK call=' | /usr/bin/grep -vc 'mismatched_rows=0$' || true)
echo "compare calls $calls nonzero $bad" > $O/cmp_summary.txt
# GOOD: SPLIT_CPU_OK x3 (random x2, real dump incl. the CPU-vs-GPU check[] cross-check) and every GPU compare call 0.
cpu_ok=$(/usr/bin/grep -c '^SPLIT_CPU_OK' $O/cpu_test.log || true)
GOOD=0; [ "$cpu_ok" = 3 ] && [ "$calls" -gt 0 ] && [ "$bad" = 0 ] && GOOD=1
echo "cpu split==stock runs $cpu_ok of 3; GOOD=$GOOD" >> $O/cmp_summary.txt
if [ $GOOD = 1 ]; then
  echo "B" > $O/phase
  leaf $H1 170 b_warm
  for h in 300000 200000; do
    leaf $H1 $h b_stock HAZYNC_EVAL_CHECK_LOG=1
    for v in split; do leaf $H1 $h b_$v HAZYNC_EVAL_CHECK=$v HAZYNC_EVAL_CHECK_LOG=1; done
  done
  leaf $H0 300000 b_rel
  if [ -n "$NSYS" ]; then
    for v in stock split; do
      ( cd $O && once "nsys $v 300000" $O/prof/nsys_${v}_300000.log env HAZYNC_EVAL_CHECK=$v HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_300000.bin \
          $NSYS profile -t cuda -s none --cpuctxsw=none -o $O/prof/nsys_${v}_300000 -f true $H1 libzync-prove-range 300000 300000 unused leaf ns${v}_300000 )
    done
  fi
  ncu_run br_stock_evalcheck $H1 '^eval_check$|eval_check' 1
  ncu_run br_split $H1 "split_piece" 5 HAZYNC_EVAL_CHECK=split
fi
echo "C" > $O/phase
for i in 1 2 3; do once "method-id rel $i" $O/fx_methodid.log $H0 method-id; done
M=$W/hz/prover/testdata/libzync/mainnet
if [ -d $M ]; then
  ( cd $O && step "fx rel 1..2 one process" $O/fx_r12.log $H0 libzync-prove-range 1 2 $M genesis fx_r12 )
  ( cd $O && step "fx rel 1 alone" $O/fx_s1.log $H0 libzync-prove-range 1 1 $M genesis fx_s1 )
  st=$(ls $O/fx_s1/state_1.json 2>/dev/null)
  [ -n "$st" ] && ( cd $O && step "fx rel 2 alone" $O/fx_s2.log $H0 libzync-prove-range 2 2 $M $st fx_s2 )
  ( cd $O && step "fx rel 1..4 one process" $O/fx_r14.log $H0 libzync-prove-range 1 4 $M genesis fx_r14 )
fi
echo "VERIFY" > $O/phase
while read h hash; do
  for d in $O/*_$h; do
    [ -f $d/block_$h.rcpt ] || continue
    $V --network mainnet --product --anchor $d/anchor_effective.json --expect "$h:$hash" $d/block_$h.rcpt > $d.verify.log 2>&1
    echo -e "$(basename $d)\trc=$?\t$(/usr/bin/grep -ao 'SWIFTSYNC_VERIFY_[A-Z]*' $d.verify.log | tail -1)\t$(stat -c %s $d/block_$h.rcpt)" >> $O/verify.tsv
  done
done < $I/block_hashes.txt
# nsys summaries (kernel table + timeline facts) for the harvest
if [ -n "$NSYS" ]; then
  for r in $O/prof/*.nsys-rep; do
    b=${r%.nsys-rep}
    $NSYS stats -q -r cuda_gpu_kern_sum -f csv -o $b $r > /dev/null 2>&1
    $NSYS export -q -t sqlite -o $b.sqlite -f true $r > /dev/null 2>&1 && python3 -I $I/nsys_facts.py $b.sqlite > $b.facts.txt 2>&1 && rm -f $b.sqlite
  done
fi
echo "DONE" > $O/phase
echo "OK" > $O/RUN_DONE
