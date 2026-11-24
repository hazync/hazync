#!/bin/bash
# gpu_multi_try.sh <bundle.tgz> <counts...>: gpu_block.sh with GPU_COUNT = each count in turn (DEPLOY_WAIT_MIN each)
# until one deploys. ⚠ SPENDS MONEY (gpu_block.sh).
B=$1; shift; HERE=$(cd "$(dirname "$0")" && pwd)
for c in "$@"; do
  GPU_COUNT=$c "$HERE/gpu_block.sh" "$B"
  L=$(ls -td "$(dirname "$(readlink -f "$B")")"/run_*/ | head -1)gpu_block.log
  grep -q "deploy failed on every GPU type" "$L" || exit 0
  echo "[$(date +%T)] no $c-GPU pod; next"
done
echo "[$(date +%T)] no pod at any count"; exit 1
