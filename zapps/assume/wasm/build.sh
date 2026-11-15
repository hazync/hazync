#!/bin/bash
# Build the assume zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/assume/wasm "${ZAPP_ASSUME_WASM_STAGE:-/tmp/zapp-assume-wasm-build}" zapp_assume_wasm.wasm \
    zapps/assume zapps/common swiftsync-verify libzync
