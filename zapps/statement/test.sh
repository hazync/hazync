#!/bin/bash
# The balance statement zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the
# library's unit tests on the real mainnet 1..200 proof, the WASM crate's frame test (natively), the CLI end to end
# (test_statement.sh: fresh keys, P2WPKH, P2TR, P2PK both key forms, refusals, the identity sources) and the same
# statement through its own .wasm (wasm/test-statement.mjs, from test_statement.sh).
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-statement
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm wasm/README.md
STATEMENT_WASM=wasm/target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm ./test_statement.sh "$T/release/zapp-statement"
echo "ZAPP_STATEMENT_TESTS OK"
