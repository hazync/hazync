#!/usr/bin/env bash
# The release's real-GPU gate: ⚠ `run` SPENDS MONEY (about $0.40-0.80, capped by --max-usd, default $2).
# Run it from the MAINTAINER'S LAPTOP, between scripts/release.sh's build and publish: the RunPod key
# (~/.hazync/runpod.key, or RUNPOD_API_KEY_FILE) lives only there. Everything is in scripts/release_gpu_gate.py;
# docs/BUILD_AND_RELEASE.md "The real-GPU gate" is the procedure.
#
#   scripts/release-gpu-gate.sh run --source hazync-coord:/root/<release checkout>/dist --host-sha256 <sha256>
#   scripts/release-gpu-gate.sh record-windows --source hazync-coord:/root/<release checkout>/dist \
#       --report gate-windows.json --receipt block_170.rcpt --log prove.log --zip hazync-host-x86_64-windows-cuda.zip
#   scripts/release-gpu-gate.sh sweep [--terminate]   # ⛔ after ANY abnormal exit: leftover hz-relgate-* pods
#   scripts/release-gpu-gate.sh check --dist dist --commit <sha>   # what release.sh runs; no network
set -euo pipefail
exec python3 "$(dirname "${BASH_SOURCE[0]}")/release_gpu_gate.py" "$@"
