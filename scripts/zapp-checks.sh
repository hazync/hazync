#!/bin/bash
# Checks every zAPP's test.sh makes the same way (docs/libzync/ZYNCAPPS.md §9). Source it from zapps/<name>/test.sh.
#
#   wasm_sha_is_the_readmes <built .wasm> <README.md>
#       the module wasm/build.sh just built is the one its README pins (`sha256 (this commit, ...) | `<hex>``): a
#       published sha nothing compares is a sha nothing guards.
#   wasm_crate_test <wasm crate dir>
#       the WASM crate's native unit tests against its COMMITTED Cargo.lock: the engine graph's, with OpenVM's patched
#       tree (engine/scripts/wasm_ovm.sh) and --locked. A plain `cargo test` re-resolved the lock without the patches
#       and the next wasm/build.sh refused it (27.11.2026).
#   c_with_core <C file> <zAPP include dir> <zAPP target dir> <zAPP library name> <output>
#       compile a C program that calls the core verifier's C ABI (swiftsync-verify-ffi, its static archive) and the
#       zAPP's in one binary. The zAPP is linked as its cdylib (lib<name>.so): two Rust static archives in one binary
#       each carry a Rust std and clash at link time, so a node patch that uses both links the zAPP's shared library.
#
# ⚠ Call each as a statement of its own (`c_with_core ...` then run the binary on the next line), never as part of an
# `a && b` list, an `if` or a `||`: bash turns errexit off inside a function called there, so a failed build or link
# went on to print TESTS OK (review 11-16). Each function also returns non-zero itself on any failed step.
# zapps/common/test.sh holds the controls: a broken link and a wrong sha must fail, a good one pass.

wasm_sha_is_the_readmes() {
    local wasm=$1 readme=$2 got want
    [ -f "$wasm" ] || { echo "::error::$wasm: no such file"; return 1; }
    got=$(sha256sum "$wasm" | cut -c1-64) || return 1
    want=$(grep -o 'sha256 (this commit, `./build.sh`) | `[0-9a-f]\{64\}`' "$readme" | grep -o '[0-9a-f]\{64\}' || true)
    if [ -z "$want" ]; then echo "::error::$readme has no 'sha256 (this commit, ./build.sh)' line"; return 1; fi
    if [ "$got" != "$want" ]; then echo "::error::$wasm is $got, $readme pins $want: rebuild and update the README"; return 1; fi
    echo "wasm sha256 $got is the one $readme pins"
}

c_with_core() {
    local c=$1 inc=$2 dir=$3 lib=$4 out=$5 repo core
    repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd) || return 1
    cargo build --release --manifest-path "$repo/swiftsync-verify-ffi/Cargo.toml" || { echo "::error::the core's C ABI did not build"; return 1; }
    core=${CARGO_TARGET_DIR:-$repo/swiftsync-verify-ffi/target}/release/libswiftsync_verify.a
    [ -f "$core" ] || { echo "::error::$core: no core static archive"; return 1; }
    [ -f "$dir/lib$lib.so" ] || { echo "::error::$dir/lib$lib.so: no zAPP shared library"; return 1; }
    rm -f "$out"
    cc -O2 -I"$inc" -I"$repo/swiftsync-verify-ffi/include" "$c" "$core" -L"$dir" -l"$lib" -Wl,-rpath,"$dir" \
       -lpthread -ldl -lm -o "$out" || { echo "::error::$c: did not compile and link beside the core's C ABI"; return 1; }
}

wasm_crate_test() {
    local crate=$1
    (
        REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd) || exit 1
        # shellcheck source=../engine/scripts/wasm_ovm.sh
        . "$REPO/engine/scripts/wasm_ovm.sh"
        ovm_make_tree
        ovm_cfg "$REPO/$OVM_TREE"
        cargo test --locked --release --manifest-path "$crate/Cargo.toml" "${OVM_CFG[@]}"
    ) || { echo "::error::$crate: its native tests failed (or its Cargo.lock is not the engine graph's)"; return 1; }
}
