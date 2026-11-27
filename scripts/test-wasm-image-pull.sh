#!/bin/bash
# The build scripts' choice of build image (their shared container block), against a mock docker and cargo: with a line
# for the image's tag in reproduce/wasm-image.digests the PUBLISHED image is pulled by that digest (once: not again while
# the local tag carries it, but again when the local tag is a local build), never built; without one, the Dockerfile is
# built locally and said so; a release (HAZYNC_WASM_REQUIRE_PUBLISHED) refuses that unless HAZYNC_WASM_ALLOW_LOCAL_IMAGE.
# Nothing is pulled, built or run for real.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
R=$W/repo; mkdir -p "$R/swiftsync-verify-wasm" "$R/reproduce" "$W/bin" "$W/state"
cp "$REPO/swiftsync-verify-wasm/build.sh" "$R/swiftsync-verify-wasm/"
cp "$REPO/reproduce/wasm.Dockerfile" "$REPO/reproduce/wasm-image-tag.sh" "$R/reproduce/"
# the OpenVM tree (engine/scripts/wasm_ovm.sh) is not what this tests: a stand-in that makes, checks and patches nothing
mkdir -p "$R/engine/scripts"
cat > "$R/engine/scripts/wasm_ovm.sh" <<'O'
OVM_FILES=""; OVM_TREE=target/none; OVM_CFG=()
ovm_make_tree() { :; }; ovm_cfg() { OVM_CFG=(); }; ovm_stage() { :; }; ovm_build_or_lock() { :; }
O
# cargo metadata: a graph with no C (WITH_C=0)
printf '#!/bin/sh\necho %s\n' "'{\"packages\": [], \"resolve\": {\"nodes\": []}}'" > "$W/bin/cargo"
# docker: logs every call; `image inspect` answers from $W/state/image (absent: no such image; its content: RepoDigests)
cat > "$W/bin/docker" <<'D'
#!/bin/bash
echo "docker $*" >> "$MOCK/log"
case "$1 $2" in
  "image inspect") [ -f "$MOCK/state/image" ] || exit 1; [ "$3" = --format ] && cat "$MOCK/state/image"; exit 0;;
  "pull "*) exit 0;;
  "tag "*) echo "${2%%:*}" | grep -q @ && echo "$2" > "$MOCK/state/image"; exit 0;;
  "build "*) : > "$MOCK/state/image"; exit 0;;
  "run "*) exit 0;;
esac
exit 0
D
chmod +x "$W/bin/cargo" "$W/bin/docker"
export MOCK=$W PATH="$W/bin:$PATH"
TAG=$("$R/reproduce/wasm-image-tag.sh" 1.99.0 0)
D=sha256:$(printf '%064d' 7); REF=ghcr.io/hazync/hazync-wasm-build@$D
n=0
case_() { # case_ <name> <want exit> <digests line or -> <state image or -> <env...>
    local name=$1 want=$2 line=$3 img=$4 e; shift 4
    : > "$W/log"; rm -f "$W/state/image"
    if [ "$line" = - ]; then echo '# none' > "$R/reproduce/wasm-image.digests"; else echo "$line" > "$R/reproduce/wasm-image.digests"; fi
    [ "$img" = - ] || printf '%s' "$img" > "$W/state/image"
    set +e; (cd "$R" && env "$@" ./swiftsync-verify-wasm/build.sh) > "$W/out" 2>&1; e=$?; set -e
    [ "$e" = "$want" ] || { cat "$W/out" "$W/log"; echo "::error::$name: exit $e, want $want"; exit 1; }
    n=$((n+1))
}
has() { grep -qF -- "$1" "$W/$2" || { cat "$W/out" "$W/log"; echo "::error::$3: no '$1' in $2"; exit 1; }; }
hasnt() { ! grep -qF -- "$1" "$W/$2" || { cat "$W/out" "$W/log"; echo "::error::$3: '$1' in $2"; exit 1; }; }
case_ "a digest line: pulled by digest, not built" 0 "$TAG $D" - X=1
has "docker pull --platform linux/amd64 $REF" log "pull"; has "docker tag $REF hazync-wasm-build:$TAG" log "tag"
hasnt "docker build" log "no build"; has "docker run" log "run"; has "the published image $REF" out "said"
case_ "the local tag already carries the digest: not pulled again" 0 "$TAG $D" "$REF" X=1
hasnt "docker pull" log "no second pull"; has "docker run" log "run"
case_ "a digest line, but the local tag is a local build: pulled" 0 "$TAG $D" "" X=1
has "docker pull --platform linux/amd64 $REF" log "re-pull over a local build"
case_ "a line for another tag only: built locally, said loudly" 0 "0123456789abcdef $D" - X=1
has "docker build" log "build"; hasnt "docker pull" log "no pull"; has "building reproduce/wasm.Dockerfile LOCALLY" out "said"
case_ "a malformed digest: ignored, built locally" 0 "$TAG sha256:nothex" - X=1
has "docker build" log "build"
case_ "a release with no published image: refused" 1 - - HAZYNC_WASM_REQUIRE_PUBLISHED=1
hasnt "docker build" log "no build"; hasnt "docker run" log "no run"; has "takes no local build" out "said"
case_ "a release with the explicit override: built locally" 0 - - HAZYNC_WASM_REQUIRE_PUBLISHED=1 HAZYNC_WASM_ALLOW_LOCAL_IMAGE=1
has "docker build" log "build"
case_ "a release with a published image: pulled" 0 "$TAG $D" - HAZYNC_WASM_REQUIRE_PUBLISHED=1
has "docker pull --platform linux/amd64 $REF" log "pull"; hasnt "docker build" log "no build"
echo "WASM_IMAGE_PULL_OK: $n cases"
