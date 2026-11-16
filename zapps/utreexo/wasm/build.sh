#!/bin/bash
# Build the Utreexo zAPP checker as WASM, byte-reproducibly: scripts/wasm-build.sh, as every zAPP's (the core's flags, the
# pinned container reproduce/wasm.Dockerfile, a fixed stage path).
set -euo pipefail
cd "$(dirname "$0")"
exec ../../../scripts/wasm-build.sh zapps/utreexo/wasm /tmp/zapp-utreexo-wasm-build zapp_utreexo_wasm.wasm \
    zapps/utreexo swiftsync-verify libzync
