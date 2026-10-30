#!/bin/bash
# libzync-base.sh <dir> — build the consensus source tree a libzync guest compiles, from pinned upstream
# releases and the patch files in this repo. Nothing else edits Core: the old experimental/libzync-bench/
# hook/apply_*.py scripts produced patches/libzync/*.patch and are kept only as history.
#
#   <dir>/bitcoin-core  Bitcoin Core v28.0 (110183746) + 0001 0002 0009 0010 + libzync/0101
#   <dir>/secp256k1     libsecp256k1 v0.5.1 (642c885)  + 0012 0013 0005 0006 0008 + libzync/0102
#   <dir>/coreshim      the repo's coreshim/*.h + an empty config/bitcoin-config.h
#
# Then:  HAZYNC_BASE=<dir> HAZYNC_LIBZYNC=1 [HAZYNC_NETWORK=signet] cargo build -p host
#
# Every patch is applied with `git apply --check` first, so a drifted tree fails here, by name, instead
# of compiling into a guest that is silently missing a lever. Re-running on an existing <dir> resets the
# trees to the pinned commits first.
set -euo pipefail
DIR=${1:?usage: libzync-base.sh <dir>}
REPO=$(cd "$(dirname "$0")/.." && pwd)
CORE_TAG=v28.0;  CORE_COMMIT=110183746150428e6385880c79f8c5733b1361ba
SECP_TAG=v0.5.1; SECP_COMMIT=642c885b6102725e25623738529895a95addc4f4
mkdir -p "$DIR"

fetch() {  # fetch <subdir> <url> <tag> <commit>
  [ -d "$DIR/$1" ] || git clone -q --depth 1 -b "$3" "$2" "$DIR/$1"
  git -C "$DIR/$1" checkout -q -- . && git -C "$DIR/$1" clean -qfd
  local got; got=$(git -C "$DIR/$1" rev-parse HEAD)
  [ "$got" = "$4" ] || { echo "FATAL: $1 is at $got, pinned $4" >&2; exit 1; }
}
apply() {  # apply <subdir> <patch>...
  local d=$1; shift
  for p in "$@"; do
    git -C "$DIR/$d" apply --check "$REPO/patches/$p" || { echo "FATAL: patches/$p does not apply to $d" >&2; exit 1; }
    git -C "$DIR/$d" apply "$REPO/patches/$p"
    echo "  $d <- $p"
  done
}

fetch bitcoin-core https://github.com/bitcoin/bitcoin.git "$CORE_TAG" "$CORE_COMMIT"
fetch secp256k1 https://github.com/bitcoin-core/secp256k1.git "$SECP_TAG" "$SECP_COMMIT"
apply bitcoin-core 0001-serialize-ilp32-int-overload.patch 0002-sha256-route-through-risc0-accelerator.patch \
                   0009-sha-transform-fastpath.patch 0010-transformd64-via-accelerator.patch \
                   libzync/0101-core-libzync-levers.patch
apply secp256k1 0012-select-field-bigint2-backend.patch 0013-lift-x-via-witness-hint.patch \
                0005-ecdsa-verify-group-arith-via-bigint2.patch 0006-schnorr-verify-group-arith-via-bigint2.patch \
                0008-scalar-inverse-via-bigint2.patch libzync/0102-secp-libzync-batch-hooks.patch
mkdir -p "$DIR/coreshim/config"
: > "$DIR/coreshim/config/bitcoin-config.h"
cp "$REPO"/coreshim/*.h "$DIR/coreshim/"
echo "libzync base ready: HAZYNC_BASE=$DIR"
