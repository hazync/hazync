#!/bin/bash
# Build the liveness zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/liveness/wasm "${ZAPP_LIVENESS_WASM_STAGE:-/tmp/zapp-liveness-wasm-build}" zapp_liveness_wasm.wasm \
    zapps/liveness zapps/corecheck swiftsync-verify libzync
