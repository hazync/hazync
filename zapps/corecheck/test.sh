#!/bin/bash
# zapp-corecheck's tests (the core receipt check of every zAPP on a core proof), run from this directory (CI: the zapps
# job runs every zapps/*/test.sh), on the real mainnet 1..200 proof and the zAPP regtest harness's real receipts
# (scripts/zapp-regtest/receipts, image eb2cd424): the shipped build (--release, no feature) refuses them; the tests-only
# `regtest-test` feature (dev profile: it does not compile without debug assertions) gives OK with Core's block expected,
# FAIL with the other branch's, DEMO for a non-zero minimum work. Then the control: a release build with it is refused.
set -euo pipefail
cd "$(dirname "$0")"
cargo test --release --manifest-path ./Cargo.toml
cargo test --features regtest-test --manifest-path ./Cargo.toml
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
if cargo build --release --features regtest-test --manifest-path ./Cargo.toml > "$W/log" 2>&1; then
    echo "::error::control: a release build with the tests-only regtest-test feature compiled"; exit 1; fi
grep -q "regtest-test. feature is for tests only" "$W/log" || { tail -20 "$W/log"; echo "::error::control: the release build failed, but not on the feature's compile_error"; exit 1; }
echo "control: a release build with regtest-test does not compile"
echo "ZAPP_CORECHECK_TESTS OK"
