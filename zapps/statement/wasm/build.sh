#!/bin/bash
# Build the balance statement zAPP's WASM module byte-reproducibly, with the core verifier's flags (../../../scripts/wasm-build.sh).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/statement/wasm "${ZAPP_STATEMENT_WASM_STAGE:-/tmp/zapp-statement-wasm-build}" zapp_statement_wasm.wasm \
    zapps/statement swiftsync-verify libzync
