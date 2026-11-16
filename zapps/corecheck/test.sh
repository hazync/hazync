#!/bin/bash
# zapp-corecheck's tests (the core receipt check of every zAPP on a core proof), run from this directory (CI: the zapps
# job runs every zapps/*/test.sh), on the real mainnet 1..200 proof.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --release --manifest-path ./Cargo.toml
echo "ZAPP_CORECHECK_TESTS OK"
