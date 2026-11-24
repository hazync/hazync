#!/bin/bash
# assemble.sh <out>: the engine guest, built from THIS checkout's product guest (prover/methods/guest) and the shared
# sources it #[path]-includes (libzync, coinbase-smt), laid out in <out> with the same relative paths, plus the engine's
# edits (edits.py), its batch-signature module and manifest. The product tree is never modified. Then:
#   HZ_BASE=<base> SECP_DIR=<base>/secp256k1 hzengine build --dir <keys> --guest <out>/prover/methods/guest
# where <base> comes from base.sh.
set -euo pipefail
OUT=${1:?usage: assemble.sh <out>}
K=$(cd "$(dirname "$0")" && pwd); E=$(cd "$K/.." && pwd); R=$(cd "$E/.." && pwd)
G=$OUT/prover/methods/guest
rm -rf "$OUT/prover" "$OUT/libzync" "$OUT/coinbase-smt"
mkdir -p "$OUT/prover/methods" "$OUT/libzync" "$OUT/coinbase-smt"
cp -r "$R/prover/methods/guest" "$G"; rm -rf "$G/target" "$G/Cargo.lock" "$G/openvm-init.rs"
cp -r "$R/libzync/src" "$OUT/libzync/src"; cp -r "$R/coinbase-smt/src" "$OUT/coinbase-smt/src"
cp "$K/batch_ecdsa_ovm.rs" "$G/src/"
python3 "$K/edits.py" "$G/src/main.rs"
sed "s|@ENGINE@|$E|g" "$K/Cargo.toml.in" > "$G/Cargo.toml"
cp "$K/build.rs" "$G/build.rs"
echo "assembled $G (product $(git -C "$R" rev-parse --short HEAD))"
