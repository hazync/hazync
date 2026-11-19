#!/bin/bash
# The deposit zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests (src/tests.rs), the WASM crate's frame test (natively), the CLI's build and check on the real receipts
# (test_deposit.sh: the core proof 0..200, the header chain 0..200 and 0..975,278, Groth16-wrapped receipts in the unit
# tests) and the WASM module through its glue, its sha against the README's (wasm/test-deposit.mjs); and the OK verdict
# on the regtest harness's real receipts (test-regtest.sh, the tests-only regtest-test feature). The receipts are the experiment's
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
# the OK verdict on real proofs: the regtest harness's receipts and histories (core proof only: the header guest is
# mainnet only), Core's blocks and gettxout, under the tests-only regtest-test feature (dev profile: it does not compile
# without debug assertions); the shipped CLI and .wasm refusing them
cargo build --features regtest-test --manifest-path ./cli/Cargo.toml --bin zapp-deposit
bash test-regtest.sh "$T/debug/zapp-deposit" "$T/release/zapp-deposit" wasm/target/wasm32-unknown-unknown/release/zapp_deposit_wasm.wasm \
    || { grep -B3 WRONG test-regtest.out; tail -1 test-regtest.out; exit 1; }
tail -1 test-regtest.out
echo "ZAPP_DEPOSIT_TESTS OK"
