#!/bin/bash
# Build the WASM verifier byte-reproducibly (stage the sources at a fixed path, remap every embedded path, pinned
# Rust): ../scripts/wasm-build.sh, the one script the core and every zAPP (../zapps/<name>/wasm/build.sh) build with.
# The core verifier checks CORE chain proofs only; a zAPP is its own module and never adds code to this one.
set -euo pipefail
cd "$(dirname "$0")"
exec ../scripts/wasm-build.sh swiftsync-verify-wasm "${SWIFTSYNC_WASM_STAGE:-/tmp/swiftsync-wasm-build}" swiftsync_verify_wasm.wasm \
    swiftsync-verify libzync
