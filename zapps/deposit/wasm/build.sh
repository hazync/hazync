#!/bin/bash
# Build the deposit zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/deposit/wasm /tmp/zapp-deposit-wasm-build zapp_deposit_wasm.wasm \
    zapps/deposit zapps/headers zapps/corecheck swiftsync-verify libzync prover/zyncapps/methods/headers/src/mmr.rs
