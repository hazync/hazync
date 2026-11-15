#!/bin/bash
# zapp-common's tests (the chain-identity flags every zAPP CLI takes), run from this directory (CI: the zapps job runs
# every zapps/*/test.sh). The lookups themselves (mempool.space, your node) are exercised by each zAPP's own tests.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --release --manifest-path ./Cargo.toml
echo "ZAPP_COMMON_TESTS OK"
