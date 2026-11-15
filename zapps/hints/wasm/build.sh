#!/bin/bash
# Build the hints zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/hints/wasm "${ZAPP_HINTS_WASM_STAGE:-/tmp/zapp-hints-wasm-build}" zapp_hints_wasm.wasm \
    zapps/hints zapps/common swiftsync-verify libzync
