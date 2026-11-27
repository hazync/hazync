#!/bin/bash
# make_verify.sh <v2.0.2 tree from openvm_src.sh (OVM_VERSION=v2.0.2 OVM_SBK_PATHS=1)> <out dir>: verify-core for
# OpenVM v2.0.2 as its own crate, `hazync-engine-verify-v2-0-2`, depending on that tree by PATH: what a v2.1.0 guest
# compiles to check a v2.0.2 predecessor's proof (xver_ovm.rs, engine/HANDOVER.md step 3). The port is the verify-core
# part of engine-on-v2.0.2.patch (proof decode without the vk, the baseline's app_exe_commit field).
set -euo pipefail
T=$(readlink -f "$1"); OUT=$(readlink -m "$2"); H=$(cd "$(dirname "$0")" && pwd); R=$(cd "$H/../../.." && pwd)
rm -rf "${OUT:?}"; mkdir -p "$OUT"
(cd "$R/engine/verify-core" && tar --exclude=target --exclude=Cargo.lock --exclude=./tests -cf - .) | (cd "$OUT" && tar -xf -)
# only the verify-core hunks of the port
python3 - "$H/engine-on-v2.0.2.patch" > "$OUT/port.patch" <<'PY'
import sys, re
s = open(sys.argv[1]).read()
parts = re.split(r'(?m)^(?=--- a/)', s)
print("".join(p.replace("a/engine/verify-core/", "a/").replace("b/engine/verify-core/", "b/") for p in parts if p.startswith("--- a/engine/verify-core/")), end="")
PY
(cd "$OUT" && patch -p1 -s --batch --forward < port.patch) && rm "$OUT/port.patch"
python3 - "$OUT/Cargo.toml" "$T" <<'PY'
import sys, re
p, t = sys.argv[1], sys.argv[2]
s = open(p).read()
s = s.replace('name = "hazync-engine-verify"', 'name = "hazync-engine-verify-v2-0-2"', 1)
s = re.sub(r'^openvm-verify-stark-host = \{[^}]*\}', f'openvm-verify-stark-host = {{ path = "{t}/openvm/crates/verify", default-features = false }}', s, flags=re.M)
s = re.sub(r'^openvm-stark-sdk = \{[^}]*\}', f'openvm-stark-sdk = {{ path = "{t}/stark-backend/crates/stark-sdk", default-features = false }}', s, flags=re.M)
assert "hazync-engine-verify-v2-0-2" in s and s.count(f'path = "{t}/') == 2, s
open(p, "w").write(s)
PY
echo "verify-core for OpenVM v2.0.2 in $OUT (package hazync-engine-verify-v2-0-2, OpenVM from $T)"
