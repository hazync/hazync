#!/bin/bash
# The WASM build scripts find themselves and their checkout from ANY cwd and by any path they are called by (CI calls
# ./swiftsync-verify-wasm/build.sh from the repo root; a developer ./build.sh from its directory, or an absolute path
# from elsewhere). On 16.11.2026 the container block resolved a cwd-relative BASH_SOURCE after the script's own cd, and
# CI's libzync-tests job failed with `realpath: ./swiftsync-verify-wasm/build.sh: No such file or directory`.
# HAZYNC_WASM_DRY_RUN=1 stops each script right after it has located itself (no docker, no cargo, nothing built).
#   scripts/test-wasm-build-cwd.sh            every build script, from three cwds each and through a symlink; and
#                                             HAZYNC_WASM_IN_CONTAINER=1 on the host refused (it never skips the container)
#   scripts/test-wasm-build-cwd.sh --control  the 16.11 resolution (BASH_SOURCE after the cd) must FAIL this test
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
if [ "${1:-}" = --control ]; then
    # a copy of the checkout's build script with the old order: `cd "$(dirname "$0")"` first, BASH_SOURCE resolved after
    mkdir -p "$W/repo/swiftsync-verify-wasm" && cp -r "$REPO/reproduce" "$W/repo/" 2>/dev/null || true
    sed -e '/^SELF_ABS=/d' -e 's|^cd "$(dirname "$SELF_ABS")"$|cd "$(dirname "$0")"\nSELF_ABS=$(realpath "${BASH_SOURCE[0]}")|' \
        "$REPO/swiftsync-verify-wasm/build.sh" > "$W/repo/swiftsync-verify-wasm/build.sh"
    chmod +x "$W/repo/swiftsync-verify-wasm/build.sh"
    grep -q '^cd "$(dirname "$0")"$' "$W/repo/swiftsync-verify-wasm/build.sh" \
        || { echo "::error::control: the build script no longer has the line this control rewrites; update the control"; exit 1; }
    if (cd "$W/repo" && HAZYNC_WASM_DRY_RUN=1 ./swiftsync-verify-wasm/build.sh) > "$W/out" 2>&1; then
        cat "$W/out"; echo "::error::control: the old resolution passed: this test cannot see the bug"; exit 1; fi
    echo "control: the old resolution fails from the repo root ($(tail -1 "$W/out"))"; exit 0
fi
n=0
check() { # check <cwd> <script as called>: it must locate itself as <expect> (relative to the checkout)
    local d=$1 s=$2 want=$3 out
    out=$(cd "$d" && HAZYNC_WASM_DRY_RUN=1 "$s" 2>&1) || { echo "$out"; echo "::error::$s from $d: exit non-zero"; exit 1; }
    echo "$out" | grep -qx "dry run: .* in $REPO, re-run in the container as bash $want" \
        || { echo "$out"; echo "::error::$s from $d: did not locate itself as $want in $REPO"; exit 1; }
    n=$((n+1))
}
ZB=$(cd "$REPO" && git ls-files 'zapps/*/wasm/build.sh')
[ -n "$ZB" ] || echo "(no zapps/*/wasm/build.sh in this checkout: the core verifier's build script only)"
for b in swiftsync-verify-wasm/build.sh $ZB; do
    # a zAPP's build.sh execs scripts/wasm-build.sh, which is what re-runs in the container
    want=$b; case $b in zapps/*) want=scripts/wasm-build.sh;; esac
    check "$REPO" "./$b" "$want"
    check "$REPO/$(dirname "$b")" "./$(basename "$b")" "$want"
    check / "$REPO/$b" "$want"
done
# through a symlink elsewhere: the script resolves it, so the checkout is still the real one
ln -s "$REPO/swiftsync-verify-wasm/build.sh" "$W/linked-build.sh"
check "$W" ./linked-build.sh swiftsync-verify-wasm/build.sh
# HAZYNC_WASM_IN_CONTAINER on the host (no /etc/hazync-wasm-build-container) must refuse, never build outside the container
if [ ! -f /etc/hazync-wasm-build-container ]; then
    if (cd "$REPO" && HAZYNC_WASM_IN_CONTAINER=1 ./swiftsync-verify-wasm/build.sh) > "$W/out" 2>&1 || ! grep -q 'not the build container' "$W/out"; then
        cat "$W/out"; echo "::error::HAZYNC_WASM_IN_CONTAINER=1 on the host was not refused"; exit 1; fi
    n=$((n+1))
fi
echo "WASM_BUILD_CWD_OK: $n calls located their checkout"
