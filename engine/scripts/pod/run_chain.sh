#!/bin/bash
# On the pod: a CHAIN on the engine from genesis (FEASIBILITY Q20: the core proof the zyncAPPs verify). The product host
# (export/host, mainnet) proves blocks 1..CHAIN_TO (export/chain_to, default 200) of export/blocks through the engine
# (HAZYNC_ENGINE), from the genesis anchor. Kept in out/keep: the last state, history.bin, the last two blocks' proofs
# (renamed .pf, the harvest skips *.proof.json) and journals, and the run's timing.
cd /workspace; source ~/.cargo/env; export PATH=/usr/local/cuda-12.9/bin:$PATH
mkdir -p out/keep; X=/workspace/export; H=$X/host
chmod +x $H host/target/release/hzengine
N=$(cat $X/chain_to 2>/dev/null || echo 200)
nvidia-smi --query-gpu=name,memory.total --format=csv,noheader > out/gpu.txt
export HAZYNC_ENGINE=/workspace/host/target/release/hzengine HAZYNC_ENGINE_KEYS=/workspace/keys HAZYNC_ENGINE_GPUS=$(nvidia-smi -L | wc -l)
( nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader -l 5 > out/smi_chain.log ) & SMI=$!
s=$(date +%s)
$H libzync-prove-range 1 $N $X/blocks genesis /workspace/chain > out/chain.log 2>&1; rc=$?
e=$(( $(date +%s)-s )); kill $SMI
C=/workspace/chain
cp $C/state_$N.json $C/history.bin $C/times.tsv $C/anchor_effective.json out/keep/ 2>/dev/null
for h in $N $((N-1)); do
  cp $C/block_$h.ovm.proof.json out/keep/block_$h.ovm.pf 2>/dev/null; cp $C/block_$h.ovm.journal out/keep/ 2>/dev/null
done
echo -e "chain\trc=$rc\t${e}s\tblocks=$N\t$(grep -ac 'proved in' out/chain.log) proved" > out/times.tsv
echo done > out/RUN_DONE
