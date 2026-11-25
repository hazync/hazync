#!/bin/bash
# openvm-patches.sh <dir>: build the engine host against OpenVM (openvm + stark-backend) with engine/ci/*.patch applied.
# For each repo: copy the fetched checkout (the rev engine/host/Cargo.lock pins) under <dir>, apply its patches, and
# write [patch] entries in engine/host/.cargo/config.toml for its workspace crates (openvm-cuda-common declares `links`,
# so a mix of patched and git copies would not resolve). Used by .github/workflows/engine-windows.yml.
#   stark-backend-*.patch: vpmm_shim.cu's C++20 designated initializers as member assignments.
#   openvm-*.patch: C99 compound literals as std::array temporaries (rv32im mul/mulh/divrem, bigint); GCC's
#     -Wno-maybe-uninitialized passed only when the target is not MSVC (crates/vm, crates/recursion build.rs).
#   All are MSVC portability edits that keep the same values and the same Linux build; upstream candidates.
set -euo pipefail
DST=${1:?usage: openvm-patches.sh <dir>}
H=$(cd "$(dirname "$0")/../host" && pwd); CI=$(cd "$(dirname "$0")" && pwd)
(cd "$H" && cargo fetch)
CH=${CARGO_HOME:-$HOME/.cargo}
mkdir -p "$DST" "$H/.cargo"; : > "$H/.cargo/config.toml"
# repo <checkout-name> <git url> <lock pattern> <patch prefix> <crate dirs...>
repo() {
  local name=$1 url=$2 pat=$3 prefix=$4; shift 4
  local rev src out d
  rev=$(grep -o "${pat}#[0-9a-f]*" "$H/Cargo.lock" | head -1 | sed 's/.*#//' | cut -c1-7)
  [ -n "$rev" ] || { echo "no $name rev in Cargo.lock" >&2; exit 1; }
  src=$(ls -d "$CH"/git/checkouts/"$name"-*/"$rev" | head -1)
  [ -d "$src" ] || { echo "no checkout of $name $rev under $CH" >&2; exit 1; }
  out="$DST/$name"; rm -rf "$out"; cp -r "$src" "$out"
  for p in "$CI"/"$prefix"-*.patch; do (cd "$out" && patch -p1 --forward < "$p"); echo "$name: applied $(basename "$p")"; done
  d=$out; command -v cygpath >/dev/null && d=$(cygpath -m "$out")
  echo "[patch.\"$url\"]" >> "$H/.cargo/config.toml"
  find "$@" -name Cargo.toml -not -path "*/target/*" -not -path "*/tests/*" -not -path "*/programs/*" \
       -not -path "*/examples/*" -not -path "*/benches/*" -not -path "*/benchmarks/*" 2>/dev/null | sort | while read -r f; do
    n=$(sed -n '/^\[package\]/,/^\[/{s/^name = "\(.*\)"/\1/p}' "$f" | head -1)
    [ -n "$n" ] && echo "$n = { path = \"$d/${f#"$out"/}\" }" | sed 's#/Cargo.toml"#"#' >> "$H/.cargo/config.toml"
  done
}
repo stark-backend "https://github.com/openvm-org/stark-backend.git" 'stark-backend.git?tag=v2.1.0' stark-backend \
  "$DST/stark-backend/crates"
repo openvm "https://github.com/openvm-org/openvm.git" 'openvm.git?tag=v2.1.0' openvm \
  "$DST/openvm/crates" "$DST/openvm/extensions" "$DST/openvm/guest-libs"
echo "patch entries: $(grep -c ' = { path' "$H/.cargo/config.toml")"
