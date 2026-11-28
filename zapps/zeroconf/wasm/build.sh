#!/bin/bash
# Build the zeroconf zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/zeroconf/wasm /tmp/zapp-zeroconf-wasm-build zapp_zeroconf_wasm.wasm \
    zapps/zeroconf zapps/corecheck swiftsync-verify engine/verify-core libzync
