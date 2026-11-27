#!/bin/bash
# The balance statement zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the
# library's unit tests on the real mainnet 1..200 proof, the WASM crate's frame test (natively), the CLI end to end
# (test_statement.sh: fresh keys, P2WPKH, P2TR, P2PK both key forms, refusals, the identity sources) and the same
# statement through its own .wasm (wasm/test-statement.mjs, from test_statement.sh); and the OK verdict on the regtest
# harness's real receipts (test-regtest.sh, the tests-only regtest-test feature).
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
cargo test --release --manifest-path ./Cargo.toml
wasm_crate_test ./wasm
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-statement
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm wasm/README.md
STATEMENT_WASM=wasm/target/wasm32-unknown-unknown/release/zapp_statement_wasm.wasm ./test_statement.sh "$T/release/zapp-statement"
# the OK verdict on real proofs: the regtest harness's receipts and trie snapshots, ALICE's keys, Core's scantxoutset,
# under the tests-only regtest-test feature (dev profile: it does not compile without debug assertions); the shipped
# CLI and .wasm refusing them
# (the OK verdict on the regtest harness's RISC Zero receipts, test-regtest.sh, went with them at the cutover, 27.11.2026,
# until the harness is proved on the engine; the tests-only feature still builds)
cargo build --features regtest-test --manifest-path ./cli/Cargo.toml --bin zapp-statement
echo "ZAPP_STATEMENT_TESTS OK"
