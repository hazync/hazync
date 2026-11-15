#!/bin/bash
# The assume zAPPs' tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's unit
# tests (the ECMH against a num-bigint reference, the real snapshot), the C ABI's own tests and its C harness, the WASM
# crate's frame test (natively), the CLI end to end (test-assume.sh: Core v31.1.0's dumptxoutset files, corrupted copies,
# the identity sources, the independent reference scripts/libzync/assume_ref.py) and the WASM module through its glue.
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
cargo test --release --manifest-path ./Cargo.toml
cargo test --release --manifest-path ./ffi/Cargo.toml
cargo test --release --manifest-path ./wasm/Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-assume
bash test-assume.sh "$T/release/zapp-assume" || { tail -20 test-assume.out; exit 1; }
tail -1 test-assume.out
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_assume.c "$FT/release/libzapp_assume_ffi.a" -lpthread -ldl -lm -o "$T/atest" && "$T/atest")
./wasm/build.sh
node wasm/test-assume.mjs
echo "ZAPP_ASSUME_TESTS OK"
