#!/bin/bash
# The deposit zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests (src/tests.rs), the WASM crate's frame test (natively), the CLI's build and check on the real receipts
# (test_deposit.sh: the core proof 0..200, the header chain 0..200 and 0..975,278, Groth16-wrapped receipts in the unit
# tests) and the WASM module through its glue, its sha against the README's (wasm/test-deposit.mjs). The receipts are the experiment's
# (experimental/libzync-bench/deposit, ../headers) and the core verifier's vectors.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-deposit
bash test_deposit.sh "$T/release/zapp-deposit"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_deposit_wasm.wasm wasm/README.md
node wasm/test-deposit.mjs "$T/release/zapp-deposit"
echo "ZAPP_DEPOSIT_TESTS OK"
