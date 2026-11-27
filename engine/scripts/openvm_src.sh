#!/bin/bash
# openvm_src.sh <out dir> [patch ...]: OpenVM v2.1.0 and stark-backend v2.1.0 at the EXACT commits our lockfiles pin,
# fetched from their repos (never a local cargo checkout), each refused unless HEAD is that commit, then the named
# patches applied: `<out>/openvm`, `<out>/stark-backend`. A patch file applies to the tree its name says
# (openvm-*.patch -> openvm, stark-*.patch -> stark-backend). For builds that ship (the wasm verifier, the handover
# guest): engine/VERIFIER.md, HANDOVER.md. OVM_CACHE=<dir> keeps the clones between runs (they are re-checked).
#   engine/scripts/openvm_src.sh /tmp/ovm engine/wasm-verify/openvm-abi-stable.patch
set -euo pipefail
OUT=$(readlink -m "$1"); shift
# OVM_VERSION (default v2.1.0, what the engine builds on): the OpenVM release and the stark-backend release it pins,
# each with the exact commit its tag names (checked after the fetch). v2.0.2 is the predecessor of the step-3 handover
# rehearsal (engine/HANDOVER.md).
OVM_URL=https://github.com/openvm-org/openvm.git; SBK_URL=https://github.com/openvm-org/stark-backend.git
case "${OVM_VERSION:-v2.1.0}" in
  v2.1.0) OVM_TAG=v2.1.0; OVM_REV=05f3b73e7294fbb6d38155abafc98f1fe2bcfb73; SBK_TAG=v2.1.0; SBK_REV=a6b4761ad9a098f3a166740d22ad60b96400cd65 ;;
  v2.0.2) OVM_TAG=v2.0.2; OVM_REV=59a69b8b0cbee7011ac978e4cc07707ee3681944; SBK_TAG=v2.0.1; SBK_REV=362c7ad8c6b042b320471a137e3eadec7ec69a44 ;;
  *) echo "OVM_VERSION=$OVM_VERSION: not a version this script pins" >&2; exit 1 ;;
esac
CACHE=${OVM_CACHE:-$OUT/.cache}
fetch() { # <url> <tag> <rev> <dir>
  local url=$1 tag=$2 rev=$3 dir=$4
  [ -d "$dir/.git" ] || git clone -q --depth 1 --branch "$tag" "$url" "$dir"
  local head; head=$(git -C "$dir" rev-parse HEAD)
  [ "$head" = "$rev" ] || { echo "$url $tag is $head, not the pinned $rev: refusing" >&2; exit 1; }
  [ -z "$(git -C "$dir" status --porcelain)" ] || { echo "$dir: the cached clone has changes: refusing" >&2; exit 1; }
}
mkdir -p "$CACHE"
fetch "$OVM_URL" "$OVM_TAG" "$OVM_REV" "$CACHE/openvm"
fetch "$SBK_URL" "$SBK_TAG" "$SBK_REV" "$CACHE/stark-backend"
rm -rf "$OUT/openvm" "$OUT/stark-backend"
# the working trees without .git (a build must not see the clone's metadata)
mkdir -p "$OUT/openvm" "$OUT/stark-backend"
git -C "$CACHE/openvm" archive HEAD | tar -x -C "$OUT/openvm"
git -C "$CACHE/stark-backend" archive HEAD | tar -x -C "$OUT/stark-backend"
for p in "$@"; do
  p=$(readlink -f "$p")
  case "$(basename "$p")" in
    openvm-*) t=openvm ;;
    stark-*) t=stark-backend ;;
    *) echo "$p: name it openvm-*.patch or stark-*.patch" >&2; exit 1 ;;
  esac
  (cd "$OUT/$t" && patch -p1 -s --batch --forward --no-backup-if-mismatch < "$p") || { echo "$p did not apply to $t" >&2; exit 1; }
  echo "applied $(basename "$p") to $t ($(sha256sum "$p" | cut -c1-16))"
done
# OVM_SBK_PATHS=1: the OpenVM tree's stark-backend dependencies become PATH dependencies on the sibling stark-backend
# tree, so the tree builds with no [patch] table: a SECOND OpenVM version can then sit in a graph whose own [patch]
# tables serve the first (a guest carrying its predecessor's verifier, engine/HANDOVER.md step 3).
if [ "${OVM_SBK_PATHS:-}" = 1 ]; then
    python3 - "$OUT/openvm/Cargo.toml" "$OUT/stark-backend" <<'PY'
import re, sys, os
p, sbk = sys.argv[1], sys.argv[2]
names = {}
for d, _, fs in os.walk(os.path.join(sbk, "crates")):
    if "Cargo.toml" in fs and "/target" not in d:
        m = re.search(r'^\[package\][^\[]*?^name = "([^"]+)"', open(os.path.join(d, "Cargo.toml")).read(), re.M | re.S)
        if m: names[m.group(1)] = d
s = open(p).read(); n = 0
def sub(m):
    global n
    name = m.group(1)
    if name not in names: return m.group(0)
    n += 1
    rest = re.sub(r'git = "[^"]*",\s*(tag|branch|rev) = "[^"]*"', f'path = "{names[name]}"', m.group(2))
    return f"{name} = {{{rest}}}"
s = re.sub(r'^([a-z0-9-]+) = \{([^}\n]*github\.com/openvm-org/stark-backend[^}\n]*)\}', sub, s, flags=re.M)
open(p, "w").write(s); print(f"  {n} stark-backend dependencies of the OpenVM tree made path dependencies")
PY
fi
echo "OPENVM_SRC_OK $OUT (openvm $OVM_TAG $OVM_REV, stark-backend $SBK_TAG $SBK_REV)"
