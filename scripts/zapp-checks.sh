#!/bin/bash
# Checks every zAPP's test.sh makes the same way (docs/libzync/ZYNCAPPS.md §9). Source it from zapps/<name>/test.sh.
#
#   wasm_sha_is_the_readmes <built .wasm> <README.md>
#       the module wasm/build.sh just built is the one its README pins (`sha256 (this commit, ...) | `<hex>``): a
#       published sha nothing compares is a sha nothing guards.
#   c_with_core <C file> <zAPP include dir> <zAPP target dir> <zAPP library name> <output>
#       compile a C program that calls the core verifier's C ABI (swiftsync-verify-ffi, its static archive) and the
#       zAPP's in one binary. The zAPP is linked as its cdylib (lib<name>.so): two Rust static archives in one binary
#       each carry a Rust std and clash at link time, so a node patch that uses both links the zAPP's shared library.

wasm_sha_is_the_readmes() {
    local wasm=$1 readme=$2 got want
    got=$(sha256sum "$wasm" | cut -c1-64)
    want=$(grep -o 'sha256 (this commit, `./build.sh`) | `[0-9a-f]\{64\}`' "$readme" | grep -o '[0-9a-f]\{64\}' || true)
    if [ -z "$want" ]; then echo "::error::$readme has no 'sha256 (this commit, ./build.sh)' line"; return 1; fi
    if [ "$got" != "$want" ]; then echo "::error::$wasm is $got, $readme pins $want: rebuild and update the README"; return 1; fi
    echo "wasm sha256 $got is the one $readme pins"
}

c_with_core() {
    local c=$1 inc=$2 dir=$3 lib=$4 out=$5 repo core
    repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
    cargo build --release --manifest-path "$repo/swiftsync-verify-ffi/Cargo.toml"
    core=${CARGO_TARGET_DIR:-$repo/swiftsync-verify-ffi/target}/release/libswiftsync_verify.a
    cc -O2 -I"$inc" -I"$repo/swiftsync-verify-ffi/include" "$c" "$core" -L"$dir" -l"$lib" -Wl,-rpath,"$dir" \
       -lpthread -ldl -lm -o "$out"
}
