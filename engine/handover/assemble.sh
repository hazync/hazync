#!/bin/bash
# assemble.sh <out dir>: the handover verifier guest (HANDOVER.md step 1) with a patched OpenVM v2.1.0 beside it
# (<out>/openvm: engine/wasm-verify/openvm-abi-stable.patch + openvm-verify-nozstd.patch). Build it with
# `hzengine build --dir <keys> --guest <out>/guest`.
set -euo pipefail
OUT=$(readlink -m "$1"); HERE=$(cd "$(dirname "$0")" && pwd); ENGINE=$(cd "$HERE/.." && pwd)
SRC=${OVM_SRC:-$HOME/.cargo/git/checkouts/openvm-e4fe658744a82733/05f3b73}
rm -rf "$OUT"; mkdir -p "$OUT/guest/src"
cp -r "$SRC" "$OUT/openvm"
(cd "$OUT/openvm" && patch -p1 -s < "$ENGINE/wasm-verify/openvm-abi-stable.patch" && patch -p1 -s < "$HERE/openvm-verify-nozstd.patch")
cp "$HERE/verifier-guest/src/main.rs" "$OUT/guest/src/"
sed -e "s|@ENGINE@|$ENGINE|g" -e "s|@OVM@|$OUT/openvm|g" "$HERE/verifier-guest/Cargo.toml.in" > "$OUT/guest/Cargo.toml"
# every openvm-org/openvm crate from the ONE patched copy (the runtime and the verifier must not be two copies)
{ echo; echo '[patch."https://github.com/openvm-org/openvm.git"]'
  find "$OUT/openvm/crates" "$OUT/openvm/extensions" "$OUT/openvm/guest-libs" -name Cargo.toml -not -path "*/target/*" \
       -not -path "*/tests/*" -not -path "*/programs/*" -not -path "*/examples/*" -not -path "*/benches/*" -not -path "*/benchmarks/*" | sort |
  while read -r f; do n=$(sed -n '/^\[package\]/,/^\[/{s/^name = "\(.*\)"/\1/p}' "$f" | head -1); [ -n "$n" ] && echo "$n = { path = \"$(dirname "$f")\" }"; done
} >> "$OUT/guest/Cargo.toml"
echo "assembled $OUT/guest (OpenVM from $SRC, patched; $(grep -c 'path = ' "$OUT/guest/Cargo.toml") path entries)"
