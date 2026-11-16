#!/bin/bash
# The assume zAPPs' tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests (the ECMH against a num-bigint reference, the real snapshot), the C ABI's own tests and its C harness, the WASM
# crate's frame test (natively), the CLI end to end (test-assume.sh: Core v31.1.0's dumptxoutset files, corrupted copies,
# the identity sources, the independent reference scripts/libzync/assume_ref.py), the OK verdict on the regtest harness's
# real receipts (test-regtest.sh, the tests-only regtest-test feature) and the WASM module through its glue.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
T=${CARGO_TARGET_DIR:-$PWD/cli/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./ffi/Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./cli/Cargo.toml --bin zapp-assume
bash test-assume.sh "$T/release/zapp-assume" || { tail -20 test-assume.out; exit 1; }
tail -1 test-assume.out
# the OK verdict on real proofs: the regtest harness's receipts, under the tests-only regtest-test feature (dev profile:
# it does not compile without debug assertions), against Core's oracle; and the shipped CLI refusing them
cargo build --features regtest-test --manifest-path ./cli/Cargo.toml --bin zapp-assume
bash test-regtest.sh "$T/debug/zapp-assume" "$T/release/zapp-assume" || { grep -B3 WRONG test-regtest.out; tail -1 test-regtest.out; exit 1; }
tail -1 test-regtest.out
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_assume.c "$FT/release/libzapp_assume_ffi.a" -lpthread -ldl -lm -o "$T/atest" && "$T/atest")
c_with_core ffi/test_with_core.c ffi/include "$FT/release" zapp_assume_ffi "$T/both"
"$T/both"
./wasm/build.sh
wasm_sha_is_the_readmes wasm/target/wasm32-unknown-unknown/release/zapp_assume_wasm.wasm wasm/README.md
node wasm/test-assume.mjs
echo "ZAPP_ASSUME_TESTS OK"
