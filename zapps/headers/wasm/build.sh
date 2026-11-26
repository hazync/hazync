#!/bin/bash
# Build the header-chain zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/headers/wasm /tmp/zapp-headers-wasm-build zapp_headers_wasm.wasm \
    zapps/headers swiftsync-verify engine/verify-core libzync prover/zyncapps/methods/headers/src/mmr.rs
