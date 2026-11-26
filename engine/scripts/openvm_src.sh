#!/bin/bash
# openvm_src.sh <out dir> [patch ...]: OpenVM v2.1.0 and stark-backend v2.1.0 at the EXACT commits our lockfiles pin,
# fetched from their repos (never a local cargo checkout), each refused unless HEAD is that commit, then the named
# patches applied: `<out>/openvm`, `<out>/stark-backend`. A patch file applies to the tree its name says
# (openvm-*.patch -> openvm, stark-*.patch -> stark-backend). For builds that ship (the wasm verifier, the handover
# guest): engine/VERIFIER.md, HANDOVER.md. OVM_CACHE=<dir> keeps the clones between runs (they are re-checked).
#   engine/scripts/openvm_src.sh /tmp/ovm engine/wasm-verify/openvm-abi-stable.patch
set -euo pipefail
OUT=$(readlink -m "$1"); shift
OVM_URL=https://github.com/openvm-org/openvm.git;         OVM_REV=05f3b73e7294fbb6d38155abafc98f1fe2bcfb73
SBK_URL=https://github.com/openvm-org/stark-backend.git;  SBK_REV=a6b4761ad9a098f3a166740d22ad60b96400cd65
CACHE=${OVM_CACHE:-$OUT/.cache}
fetch() { # <url> <tag> <rev> <dir>
  local url=$1 tag=$2 rev=$3 dir=$4
  [ -d "$dir/.git" ] || git clone -q --depth 1 --branch "$tag" "$url" "$dir"
  local head; head=$(git -C "$dir" rev-parse HEAD)
  [ "$head" = "$rev" ] || { echo "$url $tag is $head, not the pinned $rev: refusing" >&2; exit 1; }
  [ -z "$(git -C "$dir" status --porcelain)" ] || { echo "$dir: the cached clone has changes: refusing" >&2; exit 1; }
}
mkdir -p "$CACHE"
fetch "$OVM_URL" v2.1.0 "$OVM_REV" "$CACHE/openvm"
fetch "$SBK_URL" v2.1.0 "$SBK_REV" "$CACHE/stark-backend"
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
  (cd "$OUT/$t" && patch -p1 -s --no-backup-if-mismatch < "$p") || { echo "$p did not apply to $t" >&2; exit 1; }
  echo "applied $(basename "$p") to $t ($(sha256sum "$p" | cut -c1-16))"
done
echo "OPENVM_SRC_OK $OUT (openvm $OVM_REV, stark-backend $SBK_REV)"
