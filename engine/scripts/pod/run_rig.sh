#!/bin/bash
# On the pod (FEASIBILITY Q7 + Q9). First Q7: the #794 attack blocks (export/attack: mem_k1140, light_only, tap_small,
# tap_full, their anchors, and RISC Zero's end states r0/<name>/state_<h>.json) through libzync-prove-range on the
# engine, end state compared byte for byte. Then Q9: the core-rules rig (experimental/libzync-bench/core_rules/rig.py) with the product host
# built HAZYNC_NETWORK=regtest proving through the ENGINE (HAZYNC_ENGINE), real proofs on this GPU, against a real
# Bitcoin Core 31.1 on regtest. The bundle's export/ carries: host_regtest, bin/{bitcoind,bitcoin-cli},
# core/test/functional/test_framework, rig.py. keys/ = the regtest engine image (keys-rt). RIG_CASES: optional subset.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out; X=/workspace/export
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
chmod +x $X/host_regtest $X/bin/* host/target/release/hzengine
export HAZYNC_ENGINE=/workspace/host/target/release/hzengine HAZYNC_ENGINE_KEYS=/workspace/keys HAZYNC_ENGINE_GPUS=1
export BITCOIND=$X/bin/bitcoind HZ_CORE_SRC=$X/core
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 5 > out/smi_rig.log ) & SMI=$!
A=$X/attack
# bundle switches (gpu_block.sh passes only JOBS/R0_BLOCKS): export/rig_only.txt = rig.py's RIG_ONLY (comma-separated
# case names; the base chain is still built), export/skip_q7 = no attack blocks
[ -f $X/rig_only.txt ] && export RIG_ONLY=$(tr -d " \n" < $X/rig_only.txt)
[ -f $X/skip_q7 ] && export SKIP_Q7=1
q7() { nm=$1 lo=$2 hi=$3 blk=$4 anc=$5; s=$(date +%s)
  $X/host_regtest libzync-prove-range $lo $hi $blk $anc out/q7_$nm > out/q7_$nm.log 2>&1; rc=$?
  r=$A/r0/${nm%_m*}/state_$hi.json; m=no-r0
  [ -f $r ] && { cmp -s out/q7_$nm/state_$hi.json $r && m=STATE_IDENTICAL || m=STATE_DIFFERENT; }
  echo -e "q7_$nm\trc=$rc\t$(( $(date +%s)-s ))s\t$m\t$(grep -aci 'out of memory' out/q7_$nm.log) oom lines" >> out/times.tsv; }
# the key sets to try (attack/keysets, "bits:dir ..."; default: the bundle's keys as 2^26 and export/keys29 as 2^29), the
# anchors (attack/anchors/{wc,tap}/anchor.json when present: node format 2; else the blocks' own), and attack/skip_mem
# (mem_k1140's heavy pieces left to a multi-GPU run)
WCA=$A/blocks/anchor.json; TAPA=$A/tap/anchor.json
[ -f $A/anchors/wc/anchor.json ] && WCA=$A/anchors/wc/anchor.json
[ -f $A/anchors/tap/anchor.json ] && TAPA=$A/anchors/tap/anchor.json
if [ -z "${SKIP_Q7:-}" ] && [ -d $A ]; then
  for ks in $(cat $A/keysets 2>/dev/null || echo "26:/workspace/keys 29:$X/keys29"); do
    export HAZYNC_ENGINE_KEYS=${ks#*:}; b=${ks%%:*}
    [ -f $A/skip_mem ] || q7 mem_k1140_m$b 126 126 $A/blocks/mem_k1140 $WCA
    q7 light_only_m$b 126 126 $A/blocks/light_only $WCA
    q7 tap_small_m$b 126 127 $A/tap/tap_small $TAPA
    q7 tap_full_m$b 126 127 $A/tap/tap_full $TAPA
  done
  export HAZYNC_ENGINE_KEYS=/workspace/keys
fi
s=$(date +%s)
python3 $X/rig.py $X/host_regtest /workspace/rigroot ${RIG_CASES:-} > out/rig.log 2>&1; rc=$?
kill $SMI
echo -e "rig\trc=$rc\t$(( $(date +%s)-s ))s\t$(tail -1 out/rig.log)" >> out/times.tsv
cp -r /workspace/rigroot/out out/rig_out 2>/dev/null
echo done > out/RUN_DONE
