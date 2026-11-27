#!/bin/bash
# make.sh <out dir>: a copy of what the engine builds from, on OpenVM v2.0.2 / stark-backend v2.0.1: the predecessor
# side of the step-3 handover rehearsal (engine/HANDOVER.md). <out> gets engine/{host,verify-core,platform,guest,
# scripts} and the product sources the guest assembly reads (prover/methods/guest, libzync/src, coinbase-smt/src), every
# OpenVM / stark-backend pin retagged, and engine-on-v2.0.2.patch (v2.0.2's VerificationBaseline holds one
# app_exe_commit field; its proof decodes without the vk). The checkout itself stays on v2.1.0. Then:
#   cargo build --release in <out>/engine/host; <out>/engine/guest/assemble.sh <guest dir>; hzengine keygen / build.
set -euo pipefail
OUT=$(readlink -m "$1"); H=$(cd "$(dirname "$0")" && pwd); R=$(cd "$H/../../.." && pwd)
rm -rf "${OUT:?}/engine" "${OUT:?}/prover" "${OUT:?}/libzync" "${OUT:?}/coinbase-smt"
cp_tree() { mkdir -p "$OUT/$1"; (cd "$R/$1" && tar --exclude=target --exclude=Cargo.lock -cf - .) | (cd "$OUT/$1" && tar -xf -); }
for c in engine/host engine/verify-core engine/platform engine/guest engine/scripts engine/handover engine/wasm-verify \
         prover/methods/guest libzync coinbase-smt; do cp_tree "$c"; done
find "$OUT/engine" -name "Cargo.toml" -o -name "Cargo.toml.in" | while read -r f; do
    sed -i 's|github.com/openvm-org/openvm.git", tag = "v2.1.0"|github.com/openvm-org/openvm.git", tag = "v2.0.2"|g; s|github.com/openvm-org/stark-backend.git", tag = "v2.1.0"|github.com/openvm-org/stark-backend.git", tag = "v2.0.1"|g' "$f"
done
(cd "$OUT" && patch -p1 -s < "$H/engine-on-v2.0.2.patch")
left=$(grep -rln 'tag = "v2.1.0"' "$OUT/engine" --include=Cargo.toml --include=Cargo.toml.in || true)
[ -z "$left" ] || { echo "still pinned to v2.1.0: $left" >&2; exit 1; }
echo "engine on OpenVM v2.0.2 in $OUT (host, verify-core, platform, guest assembly, product sources)"
