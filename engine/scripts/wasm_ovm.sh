#!/bin/bash
# Sourced by swiftsync-verify-wasm/build.sh and scripts/wasm-build.sh (engine/CUTOVER.md C5): every WASM module links
# engine/verify-core, so every build takes OpenVM and stark-backend v2.1.0 patched (abi-stable, nozstd, nologging), made
# on the HOST by engine/scripts/openvm_src.sh from the pinned commits (the container has no git) into one tree shared by
# the modules ($OVM_TREE, under the checkout's ignored target/), and used only with the tree digest below, which the
# CONTAINER recomputes before staging it (engine/scripts/tree_digest.sh).
# shellcheck disable=SC2034 # these are for the scripts that source this one
OVM_PATCHES="engine/wasm-verify/openvm-abi-stable.patch engine/handover/openvm-verify-nozstd.patch engine/handover/stark-sdk-nologging.patch"
# shellcheck disable=SC2034
OVM_DIGEST=38eccd0935910175840b4644340fb1b788b5c13a0818c57f46b2503b0547fa6e
# shellcheck disable=SC2034
OVM_TREE=target/wasm-openvm-src
# the files this one names, for a caller's STAGED list (its clean-checkout check)
# shellcheck disable=SC2034
OVM_FILES="engine/scripts/wasm_ovm.sh engine/scripts/openvm_src.sh engine/scripts/tree_digest.sh $OVM_PATCHES"

# ovm_make_tree: on the host, make (or re-check) the tree; refuse any other digest.
ovm_make_tree() {
    # shellcheck disable=SC2086 # the patch list
    (cd "$REPO" && OVM_CACHE="$REPO/target/wasm-openvm-cache" engine/scripts/openvm_src.sh "$REPO/$OVM_TREE" $OVM_PATCHES >&2)
    ovm_check_tree "$REPO/$OVM_TREE"
}
# ovm_check_tree <dir>
ovm_check_tree() {
    local got; got=$("$REPO/engine/scripts/tree_digest.sh" "$1")
    [ "$got" = "$OVM_DIGEST" ] || { echo "$0: the patched OpenVM tree $1 has digest $got, not the pinned $OVM_DIGEST: refusing" >&2; exit 1; }
}
# ovm_cfg <tree root>: OVM_CFG, the cargo --config [patch] arguments: OpenVM's verifier crate and every stark-backend
# crate from that tree (absolute paths: the same lock on the host and in the container; path packages carry no path in it)
ovm_cfg() {
    local root=$1 f n d
    OVM_CFG=(--config "patch.\"https://github.com/openvm-org/openvm.git\".openvm-verify-stark-host.path=\"$root/openvm/crates/verify\"")
    while read -r f; do
        n=$(sed -n '/^\[package\]/,/^\[/{s/^name = "\(.*\)"/\1/p}' "$f" | head -1)
        d=$(dirname "$f")
        [ -n "$n" ] && OVM_CFG+=(--config "patch.\"https://github.com/openvm-org/stark-backend.git\".$n.path=\"$d\"")
    done < <(find "$root/stark-backend/crates" -name Cargo.toml -not -path "*/tests/*" -not -path "*/benches/*" | LC_ALL=C sort)
}
# ovm_stage: in the container, check the tree and stage it at $STAGE/openvm-src; sets OVM_CFG for it and the refusing
# random source's cfg (no randomness in verification: each module's no_rng)
ovm_stage() {
    ovm_check_tree "$REPO/$OVM_TREE"
    mkdir -p "$STAGE/openvm-src"
    (cd "$REPO/$OVM_TREE" && tar --exclude=./.cache -cf - .) | (cd "$STAGE/openvm-src" && tar -xf -)
    ovm_cfg "$STAGE/openvm-src"
    export RUSTFLAGS="$RUSTFLAGS --cfg getrandom_backend=\"custom\""
}
# ovm_build_or_lock <crate dir in the stage> <wasm file>: HAZYNC_WASM_WRITE_LOCK=1 resolves the staged lock against the
# patched tree (cargo keeps every version already locked) and writes it to $OUT/Cargo.lock instead of building
ovm_build_or_lock() {
    if [ -n "${HAZYNC_WASM_WRITE_LOCK:-}" ]; then
        (cd "$1" && cargo metadata --format-version 1 --filter-platform wasm32-unknown-unknown "${OVM_CFG[@]}" > /dev/null)
        cp "$1/Cargo.lock" "$OUT/Cargo.lock"
        echo "wrote the engine graph's lock: copy $OUT/Cargo.lock to the crate's Cargo.lock"
        exit 0
    fi
    (cd "$1" && cargo build --locked --release --target wasm32-unknown-unknown "${OVM_CFG[@]}")
}
