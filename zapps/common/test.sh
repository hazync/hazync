#!/bin/bash
# zapp-common's tests (the chain-identity flags every zAPP CLI takes), run from this directory (CI: the zapps job runs
# every zapps/*/test.sh). The lookups themselves (mempool.space, your node) are exercised by each zAPP's own tests.
# Also the controls of scripts/zapp-checks.sh, which every zAPP's test.sh uses: each check must FAIL on what it guards
# against (a C program that does not link beside the core's C ABI, a .wasm whose sha is not the README's) and pass on
# the good case, so a check that cannot fail is caught here.
set -euo pipefail
cd "$(dirname "$0")"
. ../../scripts/zapp-checks.sh
cargo test --release --manifest-path ./Cargo.toml
# the tests-only regtest-test feature (dev profile), and its controls: no release build with it, no WASM module with it
cargo test --features regtest-test --manifest-path ./Cargo.toml
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
if cargo build --release --features regtest-test --manifest-path ./Cargo.toml > "$W/log" 2>&1; then
    echo "::error::control: a release build of zapp-common with regtest-test compiled"; exit 1; fi
grep -q "regtest-test. feature is for tests only" "$W/log" || { tail -20 "$W/log"; echo "::error::control: not refused by the feature's compile_error"; exit 1; }
REPO=$(cd ../.. && pwd)
mkdir -p "$W/rt/src" && : > "$W/rt/src/lib.rs"
printf '[package]\nname = "rt"\nversion = "0.1.0"\nedition = "2021"\n[lib]\ncrate-type = ["cdylib"]\n[dependencies]\nzapp-common = { path = "%s/zapps/common", features = ["regtest-test"] }\n[workspace]\n' "$REPO" > "$W/rt/Cargo.toml"
# a Cargo.lock, as every real crate has (the guard reads the graph --locked and refuses a crate without one): the engine
# graph's, resolved against OpenVM's patched tree as every module's is (engine/scripts/wasm_ovm.sh, 27.11.2026)
# shellcheck source=../../engine/scripts/wasm_ovm.sh
. "$REPO/engine/scripts/wasm_ovm.sh"
ovm_make_tree
ovm_cfg "$REPO/$OVM_TREE"
cargo generate-lockfile --manifest-path "$W/rt/Cargo.toml" "${OVM_CFG[@]}" > "$W/log" 2>&1 || { cat "$W/log"; echo "::error::control: no Cargo.lock for the stand-in crate"; exit 1; }
if ../../scripts/wasm-build.sh "$(realpath --relative-to="$REPO" "$W/rt")" "$W/stage" rt.wasm > "$W/log" 2>&1; then
    echo "::error::control: scripts/wasm-build.sh built a WASM graph with regtest-test"; exit 1; fi
grep -q "enables the tests-only feature regtest-test" "$W/log" || { tail -20 "$W/log"; echo "::error::control: wasm-build.sh failed, but not on the regtest-test check"; exit 1; }
echo "control: regtest-test is refused in a release build and in a WASM build"
# a stand-in zAPP shared library, and two programs: one that links beside the core's archive, one that calls a symbol
# nothing defines
printf 'int zc_dummy(void) { return 0; }\n' > "$W/d.c"; cc -shared -fPIC -o "$W/libzc_dummy.so" "$W/d.c"
printf '#include <string.h>\n#include "swiftsync_verify.h"\nint zc_dummy(void);\nint main(void) { const char* i = swiftsync_image_id(0); return (i ? strlen(i) != 64 : 0) + zc_dummy(); }\n' > "$W/ok.c"
printf 'int zc_no_such_symbol(void);\nint main(void) { return zc_no_such_symbol(); }\n' > "$W/broken.c"
c_with_core "$W/ok.c" "$W" "$W" zc_dummy "$W/ok"
"$W/ok"
echo "control: a program beside the core's C ABI links and runs"
# as a zAPP's test.sh calls it (a statement of its own under set -e): a broken link must stop the script
if bash -c 'set -euo pipefail; . "$1"; c_with_core "$2/broken.c" "$2" "$2" zc_dummy "$2/broken"; echo REACHED' _ \
        ../../scripts/zapp-checks.sh "$W" > "$W/log" 2>&1; then
    cat "$W/log"; echo "::error::control: a test script went on after a broken link"; exit 1; fi
grep -q 'did not compile and link' "$W/log" && ! grep -q REACHED "$W/log" || { cat "$W/log"; echo "::error::control: the link failure is not named, or the script went on"; exit 1; }
[ ! -e "$W/broken" ] || { echo "::error::control: a binary was left behind"; exit 1; }
# and no zAPP calls these checks where bash drops errexit (an && / || list, an if): that is how a failed link printed
# TESTS OK (review 11-16)
if grep -nE '(c_with_core|wasm_sha_is_the_readmes)[^#]*(&&|\|\|)|(if|&&|\|\||!) *(c_with_core|wasm_sha_is_the_readmes)' ../*/test.sh | grep -v '^\.\./common/'; then
    echo "::error::a zAPP's test.sh calls a zapp-checks.sh check in an && / || list or a condition: call it on a line of its own"; exit 1; fi
echo "control: a broken link fails c_with_core"
# the sha check: a match passes, a mismatch and a README without the line fail
printf 'x' > "$W/m.wasm"; h=$(sha256sum "$W/m.wasm" | cut -c1-64)
printf '| sha256 (this commit, `./build.sh`) | `%s` |\n' "$h" > "$W/good.md"
printf '| sha256 (this commit, `./build.sh`) | `%064d` |\n' 0 > "$W/bad.md"
printf 'no sha here\n' > "$W/none.md"
wasm_sha_is_the_readmes "$W/m.wasm" "$W/good.md" > /dev/null
for r in bad none; do
    if wasm_sha_is_the_readmes "$W/m.wasm" "$W/$r.md" > /dev/null; then echo "::error::control: the sha check passed against $r.md"; exit 1; fi
done
echo "control: the sha check fails on a wrong or missing pin"
echo "ZAPP_COMMON_TESTS OK"
