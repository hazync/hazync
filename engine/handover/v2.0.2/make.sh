#!/bin/bash
# make.sh <out dir>: engine/host and engine/verify-core on OpenVM v2.0.2 / stark-backend v2.0.1, for the predecessor
# side of the step-3 handover rehearsal (engine/HANDOVER.md): copies of this checkout's crates, retagged, with
# engine-on-v2.0.2.patch (v2.0.2's VerificationBaseline holds one app_exe_commit field; its proof decodes without the
# vk). The checkout itself stays on v2.1.0. Then: cargo build --release in <out>/engine/host.
set -euo pipefail
OUT=$(readlink -m "$1"); H=$(cd "$(dirname "$0")" && pwd); R=$(cd "$H/../../.." && pwd)
rm -rf "${OUT:?}/engine"; mkdir -p "$OUT/engine"
for c in host verify-core; do mkdir -p "$OUT/engine/$c"; (cd "$R/engine/$c" && tar --exclude=target --exclude=Cargo.lock -cf - .) | (cd "$OUT/engine/$c" && tar -xf -); done
sed -i 's|github.com/openvm-org/openvm.git", tag = "v2.1.0"|github.com/openvm-org/openvm.git", tag = "v2.0.2"|g; s|github.com/openvm-org/stark-backend.git", tag = "v2.1.0"|github.com/openvm-org/stark-backend.git", tag = "v2.0.1"|g' \
    "$OUT/engine/host/Cargo.toml" "$OUT/engine/verify-core/Cargo.toml"
(cd "$OUT" && patch -p1 -s < "$H/engine-on-v2.0.2.patch")
echo "engine on OpenVM v2.0.2 in $OUT/engine (host, verify-core)"
