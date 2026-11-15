#!/bin/bash
# Build the header-chain zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/headers/wasm "${ZAPP_HEADERS_WASM_STAGE:-/tmp/zapp-headers-wasm-build}" zapp_headers_wasm.wasm \
    zapps/headers zapps/common swiftsync-verify libzync prover/zyncapps/methods/headers/src/mmr.rs
