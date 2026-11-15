#!/bin/bash
# The liveness zAPP's tests, run from this directory (CI: the zapps job runs every zapps/*/test.sh): the library's and
# the CLI's tests (tests/liveness.rs, the /live route), the CLI and its producer (vectors/run.sh), the C ABI and the
# WASM module (through its glue), all on the real mainnet 1..200 proof (8888d59c) and this zAPP's proofs in vectors/.
set -euo pipefail
cd "$(dirname "$0")"
T=${CARGO_TARGET_DIR:-$PWD/target}
FT=${CARGO_TARGET_DIR:-$PWD/ffi/target}
cargo test --release --manifest-path ./Cargo.toml
cargo build --release --manifest-path ./Cargo.toml --bin zapp-liveness
bash vectors/run.sh "$T/release/zapp-liveness"
cargo build --release --manifest-path ./ffi/Cargo.toml
(cd ffi && cc -O2 -Iinclude test_liveness.c "$FT/release/libzapp_liveness_ffi.a" -lpthread -ldl -lm -o "$T/ltest" && "$T/ltest")
./wasm/build.sh
node wasm/test-liveness.mjs
echo "ZAPP_LIVENESS_TESTS OK"
