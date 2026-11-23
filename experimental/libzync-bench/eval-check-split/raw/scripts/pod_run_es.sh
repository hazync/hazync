#!/bin/bash
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
{ nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader; nproc; grep -m1 'model name' /proc/cpuinfo; free -g | sed -n 2p; df -h /workspace | tail -1; } > $O/pod.txt
get() {  # get <release> <dir> <asset...>
  local v=$1 d=$2; shift 2; mkdir -p $d
  for f in SHA256SUMS.txt "$@"; do for t in 1 2 3; do [ -s $d/$f ] && break
    curl -fsSL -o $d/$f.tmp https://github.com/hazync/hazync/releases/download/$v/$f && mv $d/$f.tmp $d/$f; done; done; }
echo "SETUP" > $O/phase
get v0.27.1 $W/rel hazync-host-x86_64-linux-gnu-cuda hazync-host-x86_64-linux-gnu hazync-verify-x86_64-linux-gnu
[ "$(sha256sum rel/SHA256SUMS.txt | cut -c1-64)" = "$(cat in/want_sums_v0271.txt)" ] || { echo "SUMS_BAD" > $O/RUN_DONE; exit 1; }
( cd rel && sha256sum -c --ignore-missing SHA256SUMS.txt ) > $O/sha_rel.txt 2>&1 || { echo "SHA_BAD" > $O/RUN_DONE; exit 1; }
[ "$(/usr/bin/grep -c ': OK$' $O/sha_rel.txt)" = 3 ] || { echo "SHA_BAD count" > $O/RUN_DONE; exit 1; }
chmod +x rel/hazync-*; H0=$W/rel/hazync-host-x86_64-linux-gnu-cuda; V=$W/rel/hazync-verify-x86_64-linux-gnu
$H0 method-id > $O/method_id_rel.txt 2>&1; /usr/bin/grep -q 46666b6f7e3d1c1c $O/method_id_rel.txt || { echo "MID_BAD rel" > $O/RUN_DONE; exit 1; }
# the canonical guest ELF, cut out of the release CPU host (embedded verbatim) and checked by its pinned sha256
python3 -I $I/extract_elf.py rel/hazync-host-x86_64-linux-gnu $W/elf/mainnet.bin f7b87bf118050f60c45d605e2cc953668221dff95ed43072d4fd717380c08a27 > $O/elf.txt 2>&1 \
  || { echo "ELF_BAD" > $O/RUN_DONE; exit 1; }
mkdir -p $W/elf/pins; echo 46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c > $W/elf/pins/METHOD_ID
echo f7b87bf118050f60c45d605e2cc953668221dff95ed43072d4fd717380c08a27 > $W/elf/pins/GUEST_ELF_SHA256
echo "libzync=1 assumevalid=0 network=mainnet coinlog=1 trie=1 utxohash=1" > $W/elf/pins/PROFILE
# CUDA 12.8 (provision-vps.sh phase 7's package): nvcc for the build, nsys + ncu for Blackwell
once "cuda 12.8 install" $O/cuda_install.log bash -c '
  . /etc/os-release; curl -fsSL -o /tmp/kr.deb https://developer.download.nvidia.com/compute/cuda/repos/ubuntu${VERSION_ID/./}/x86_64/cuda-keyring_1.1-1_all.deb &&
  dpkg -i /tmp/kr.deb && apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq cuda-toolkit-12-8'
NSYS=$(ls /usr/local/cuda-12.8/bin/nsys /opt/nvidia/nsight-systems/*/bin/nsys 2>/dev/null | head -1)
NCU=$(ls /usr/local/cuda-12.8/bin/ncu /opt/nvidia/nsight-compute/*/ncu 2>/dev/null | head -1)
CUOBJ=/usr/local/cuda-12.8/bin/cuobjdump
echo "nsys=$NSYS ncu=$NCU" >> $O/pod.txt
( nvidia-smi --query-gpu=utilization.gpu,memory.used,power.draw --format=csv,noheader -l 5 > $O/smi.log 2>/dev/null ) &

leaf() {  # leaf <host> <h> <tag> [VAR=val...]: prove leaf h into $O/<tag>_<h>
  local H=$1 h=$2 tag=$3; shift 3; rm -rf $O/${tag}_$h
  ( cd $O && step "$tag leaf $h" $O/${tag}_$h.log env "$@" HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_$h.bin $H libzync-prove-range $h $h unused leaf ${tag}_$h ); }

echo "A" > $O/phase
leaf $H0 170 a_warm
for h in 170 300000 200000; do leaf $H0 $h a_rel; done

echo "BUILD" > $O/phase
( set -o pipefail
  git clone -q --depth 1 -b $BRANCH https://github.com/hazync/hazync $W/hz && git -C $W/hz log -1 --format='%H %s' > $O/build_commit.txt &&
  cd $W/hz && GPU=1 HAZYNC_CUDA_VER=12.8 REPO_DIR=$W/hz HAZYNC_GUEST_ELF=$W/elf/mainnet.bin HAZYNC_GUEST_PIN_DIR=$W/elf/pins \
    nice -n 5 bash ./provision-vps.sh > $O/build.log 2>&1; echo $? > $O/BUILD_RC ) &
BPID=$!
echo "A_PROF" > $O/phase
$CUOBJ --dump-resource-usage $H0 2>/dev/null | python3 -I $I/resusage.py > $O/prof/resusage_rel.txt 2>&1
if [ -n "$NSYS" ]; then
  ( cd $O && once "nsys rel 300000" $O/prof/nsys_rel_300000.log env HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_300000.bin \
      $NSYS profile -t cuda -s none --cpuctxsw=none -o $O/prof/nsys_rel_300000 -f true $H0 libzync-prove-range 300000 300000 unused leaf nsr_300000 )
  ( cd $O && once "nsys rel 170" $O/prof/nsys_rel_170.log env HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_170.bin \
      $NSYS profile -t cuda,osrt -s none --cpuctxsw=none -o $O/prof/nsys_rel_170 -f true $H0 libzync-prove-range 170 170 unused leaf nsr_170 )
fi
NCUM=launch__registers_per_thread,launch__grid_size,launch__block_size,launch__occupancy_limit_registers,sm__warps_active.avg.pct_of_peak_sustained_active,gpu__time_duration.sum,l1tex__t_bytes_pipe_lsu_mem_local_op_ld.sum,l1tex__t_bytes_pipe_lsu_mem_local_op_st.sum,l1tex__t_bytes_pipe_lsu_mem_global_op_ld.sum,dram__bytes_read.sum,dram__bytes_write.sum,smsp__thread_inst_executed.sum,launch__stack_size
ncu_run() {  # ncu_run <tag> <host> <kernel regex> <count> [VAR=val...]
  local tag=$1 H=$2 k=$3 c=$4; shift 4; [ -n "$NCU" ] || return 1
  ( cd $O && once "ncu $tag" $O/prof/ncu_$tag.log env "$@" HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_170.bin timeout 2400 $NCU --replay-mode application \
      --target-processes all -k "regex:$k" -c $c --metrics $NCUM --csv --log-file $O/prof/ncu_$tag.csv $H libzync-prove-range 170 170 unused leaf ncu_${tag}_170 ) ; }
ncu_run rel_evalcheck $H0 '^eval_check$|eval_check' 1
ncu_run rel_stepexec $H0 'stepExec' 1
wait $BPID
echo "build rc $(cat $O/BUILD_RC 2>/dev/null)" >> $O/pod.txt
H1=$W/hz/prover/target/release/host
[ "$(cat $O/BUILD_RC 2>/dev/null)" = 0 ] && [ -x $H1 ] || { echo "BUILD_FAILED" > $O/phase; }
GOOD=0
if [ -x $H1 ]; then
  $H1 method-id > $O/method_id_branch.txt 2>&1
  $CUOBJ --dump-resource-usage $H1 2>/dev/null | python3 -I $I/resusage.py > $O/prof/resusage_branch.txt 2>&1
  if /usr/bin/grep -q 46666b6f7e3d1c1c $O/method_id_branch.txt; then
    echo "CPUREAL" > $O/phase
    mkdir -p $O/dump170
    leaf $H1 170 dump HAZYNC_SEG_PO2=16 HAZYNC_EVAL_CHECK_DUMP=$O/dump170
    # fallback: the shipped po2 (a 2^23-row domain, ~7 GB of inputs) if po2 16 gave no dump
    [ -s $O/dump170/meta.txt ] || leaf $H1 170 dump21 HAZYNC_EVAL_CHECK_DUMP=$O/dump170 HAZYNC_EVAL_CHECK_DUMP_MAX_DOMAIN=8388608
    ( export PATH=$HOME/.cargo/bin:$PATH; cd $W/hz/vendor/risc0-circuit-rv32im-sys &&
      once "cpu test random" $O/cpu_test.log split/test_split_cpu.sh $W/cpub &&
      once "cpu test real" $O/cpu_test.log $W/cpub/test_split_cpu dump $O/dump170 10 )
    # the real-input dump is ~0.3 GB; keep only its meta for the harvest
    du -sh $O/dump170 >> $O/cpu_test.log; mv $O/dump170 $W/dump170
    echo "CMP" > $O/phase
    for v in split splitim chainim; do leaf $H1 300000 cmp_$v HAZYNC_EVAL_CHECK=compare:$v; done
    calls=$(cat $O/cmp_*_300000.log | /usr/bin/grep -c 'HAZYNC_EVAL_CHECK call=' || true)
    bad=$(cat $O/cmp_*_300000.log | /usr/bin/grep 'HAZYNC_EVAL_CHECK call=' | /usr/bin/grep -vc 'mismatched_rows=0$' || true)
    echo "compare calls $calls nonzero $bad" > $O/cmp_summary.txt
    /usr/bin/grep -q 'rows .*MISMATCH on 0 rows' $O/cpu_test.log && /usr/bin/grep -q '^SPLIT_CPU_OK' $O/cpu_test.log \
      && [ "$(/usr/bin/grep -c '^SPLIT_CPU_OK' $O/cpu_test.log)" = 3 ] && [ "$calls" -gt 0 ] && [ "$bad" = 0 ] && GOOD=1
    echo "GOOD=$GOOD" >> $O/cmp_summary.txt
  fi
fi
if [ $GOOD = 1 ]; then
  echo "B" > $O/phase
  leaf $H1 170 b_warm
  for h in 300000 200000; do
    leaf $H1 $h b_stock HAZYNC_EVAL_CHECK_LOG=1
    for v in split splitim chainim; do leaf $H1 $h b_$v HAZYNC_EVAL_CHECK=$v HAZYNC_EVAL_CHECK_LOG=1; done
  done
  leaf $H0 300000 b_rel
  if [ -n "$NSYS" ]; then
    for v in stock split splitim chainim; do
      ( cd $O && once "nsys $v 300000" $O/prof/nsys_${v}_300000.log env HAZYNC_EVAL_CHECK=$v HAZYNC_LEAF_BUNDLE=$I/leaves/leaf_300000.bin \
          $NSYS profile -t cuda -s none --cpuctxsw=none -o $O/prof/nsys_${v}_300000 -f true $H1 libzync-prove-range 300000 300000 unused leaf ns${v}_300000 )
    done
  fi
  ncu_run br_stock_evalcheck $H1 '^eval_check$|eval_check' 1
  for v in split splitim; do ncu_run br_$v $H1 "${v}_piece" 5 HAZYNC_EVAL_CHECK=$v; done
  ncu_run br_chainim $H1 "chainim_piece" 1 HAZYNC_EVAL_CHECK=chainim
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
