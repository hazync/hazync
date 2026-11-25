#!/bin/bash
# openvm-patches.sh <dir>: build the engine host against OpenVM's stark-backend with engine/ci/*.patch applied.
# Copies the fetched stark-backend checkout (the rev engine/host/Cargo.lock pins) to <dir>, applies the patches, and
# writes engine/host/.cargo/config.toml [patch] entries for EVERY crate of that workspace (openvm-cuda-common declares
# `links`, so a mix of patched and git copies would not resolve). Used by .github/workflows/engine-windows.yml.
#   stark-backend-msvc.patch: vpmm_shim.cu's C++20 designated initializers as member assignments (MSVC rejects
#   them under --std=c++17; GCC accepts them as an extension). Same values; an upstream candidate.
set -euo pipefail
DST=${1:?usage: openvm-patches.sh <dir>}
H=$(cd "$(dirname "$0")/../host" && pwd); CI=$(cd "$(dirname "$0")" && pwd)
REV=$(grep -o 'stark-backend.git?tag=v2.1.0#[0-9a-f]*' "$H/Cargo.lock" | head -1 | sed 's/.*#//' | cut -c1-7)
[ -n "$REV" ] || { echo "no stark-backend rev in Cargo.lock" >&2; exit 1; }
(cd "$H" && cargo fetch)
CH=${CARGO_HOME:-$HOME/.cargo}
SRC=$(ls -d "$CH"/git/checkouts/stark-backend-*/"$REV" | head -1)
[ -d "$SRC" ] || { echo "no checkout of stark-backend $REV under $CH" >&2; exit 1; }
rm -rf "$DST"; cp -r "$SRC" "$DST"
for p in "$CI"/stark-backend-*.patch; do (cd "$DST" && patch -p1 --forward < "$p"); echo "applied $(basename "$p")"; done
D=$DST; command -v cygpath >/dev/null && D=$(cygpath -m "$DST")
mkdir -p "$H/.cargo"
{
  echo '[patch."https://github.com/openvm-org/stark-backend.git"]'
  for c in stark-backend stark-sdk cpu-backend cuda-backend cuda-builder cuda-common; do
    n=$(sed -n 's/^name = "\(.*\)"/\1/p' "$DST/crates/$c/Cargo.toml" | head -1)
    echo "$n = { path = \"$D/crates/$c\" }"
  done
} > "$H/.cargo/config.toml"
cat "$H/.cargo/config.toml"
