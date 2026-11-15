#!/bin/bash
# The header-chain zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's
# unit tests (real receipts, MMR paths, transaction inclusion), the WASM crate's frame tests (natively), the CLI on the
# real chained receipts (test_headers_verify.sh) and the whole chain genesis..975,278, and the WASM module through its
# glue (wasm/test-headers.mjs). The receipts are the experiment's (experimental/libzync-bench/headers).
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-headers
bash test_headers_verify.sh "$T/release/zapp-headers"
"$T/release/zapp-headers" --expect 975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3 \
    ../../experimental/libzync-bench/headers/headers_0_tip.rcpt
./wasm/build.sh
node wasm/test-headers.mjs
echo "ZAPP_HEADERS_TESTS OK"
