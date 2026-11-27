#!/bin/bash
# engine-ids.sh [--out DIR] [--rev REV]: the ENGINE (OpenVM) image ids of this checkout's COMMIT, built in reproduce/engine.Dockerfile
# (engine/ROADMAP.md 4.1, engine/results/reproduce_engine). Prints, for each image, the exe commitment, the vm commitment
# and the image id SHA-256(exe || vm), and writes them with each image.json, baseline.json and guest.vmexe hash to DIR
# (default reproduce/out-engine). Images: core (the production core guest, mainnet, default features) and zeroconf.
#
# Fixed inside the container: the commit's tracked files (`git archive HEAD`: uncommitted edits never reach an id) at
# /root/engine-kit, the C/C++ base at /hz/base, the assembled guests at /hz/asm and /hz/zc, keys at /hz/keys-*, HOME /root.
# The keys are made here by `hzengine keygen` with HZ_GUEST_MEM_BITS=29 (the production VM config, the build box's
# keys-rt29); the vm commitment depends only on them. The guests build with their pinned locks (reproduce/engine-locks,
# OPENVM_BUILD_LOCKED=1: cargo --locked), never re-resolved.
# ⛔ /root/engine-kit is part of the id: the guests' platform crates are path dependencies there, and cargo hashes a path
# dependency's location into its -C metadata (symbol names), which --remap-path-prefix does not touch. Measured on the
# build box 27.11.2026 (engine/results/reproduce_engine): the same source with the kit at /root/r41/src/engine gave exe
# 56d9b208..., at /root/engine-kit/engine 0ddcb255... (keys-ch's). The box's reference images were built from a kit at
# /root/engine-kit, so this stage is that path. The assembled guest's own path does NOT reach the id (remapped).
# Needs docker and the network (crates, OpenVM, Core, libsecp).
# A cargo cache volume (hazync-engine-cargo) keeps downloads between runs; HAZYNC_ENGINE_FRESH=1 starts from an empty one.
set -euo pipefail
SELF=$(realpath "${BASH_SOURCE[0]}")
REPO=$(cd "$(dirname "$SELF")/.." && pwd)
if [ -n "${HAZYNC_ENGINE_IN_CONTAINER:-}" ]; then
    [ -f /etc/hazync-engine-build-container ] || { echo "$0: not the build container" >&2; exit 1; }
    for v in $(compgen -e | grep "^HAZYNC_\|^HZ_"); do [ "$v" = HAZYNC_ENGINE_IN_CONTAINER ] || unset "$v"; done
    t0=$(date +%s); say(){ echo "[$(( $(date +%s)-t0 )) s] $*"; }
    K=/root/engine-kit; rm -rf /hz $K && mkdir -p /hz $K
    tar -x -f /in/src.tar -C $K
    commit=$COMMIT
    say "commit $commit (git archive); rustc $(rustc --version | cut -d' ' -f2), guest $(rustc +nightly-2026-02-28 --version | cut -d' ' -f2-4)"
    cd $K/engine/host && cargo build --release --locked -q 2>&1 | grep -v '^warning' | tail -3 || true
    E=$K/engine/host/target/release/hzengine; [ -x $E ] || { echo "hzengine did not build" >&2; exit 1; }
    say "hzengine built"
    $K/engine/guest/base.sh /hz/base > /hz/base.log 2>&1 || { tail -5 /hz/base.log; exit 1; }
    $K/engine/guest/assemble.sh /hz/asm > /hz/asm.log 2>&1 || { tail -5 /hz/asm.log; exit 1; }
    cp $K/reproduce/engine-locks/core.Cargo.lock /hz/asm/prover/methods/guest/Cargo.lock
    $K/engine/zapps/assemble_guest.sh zeroconf /hz/zc > /hz/zc-asm.log 2>&1 || { tail -5 /hz/zc-asm.log; exit 1; }
    cp $K/reproduce/engine-locks/zeroconf.Cargo.lock /hz/zc/Cargo.lock
    say "base and guests assembled"
    HZ_GUEST_MEM_BITS=29 $E keygen --dir /hz/keys > /hz/keygen.log 2>&1 || { tail -5 /hz/keygen.log; exit 1; }
    say "keygen: $(grep -a '^keygen:' /hz/keygen.log); sdk.pk $(sha256sum /hz/keys/sdk.pk | cut -c1-16) agg.vk $(sha256sum /hz/keys/agg.vk | cut -c1-16) openvm.toml $(sha256sum /hz/keys/openvm.toml | cut -c1-16)"
    mkdir -p /out
    for spec in core:/hz/asm/prover/methods/guest zeroconf:/hz/zc; do
        n=${spec%%:*}; g=${spec#*:}; k=/hz/keys-$n
        rm -rf $k && mkdir $k && cp /hz/keys/{sdk.pk,agg.vk,openvm.toml} $k/
        OPENVM_BUILD_LOCKED=1 HZ_BASE=/hz/base $E build --dir $k --guest $g > /hz/$n.build.log 2>&1 \
            || { sed 's/\x1b\[[0-9;]*m//g' /hz/$n.build.log | grep -a "^error\|error:" -A5 | head -30; exit 1; }
        id=$(python3 -c "import json,hashlib;i=json.load(open('$k/image.json'));print(hashlib.sha256(bytes.fromhex(i['exe']+i['vm'])).hexdigest())")
        exe=$(python3 -c "import json;print(json.load(open('$k/image.json'))['exe'])"); vm=$(python3 -c "import json;print(json.load(open('$k/image.json'))['vm'])")
        mkdir -p /out/$n && cp $k/image.json $k/baseline.json /out/$n/ && echo "$id" > /out/$n/image_id
        sha256sum $k/guest.vmexe | cut -d' ' -f1 > /out/$n/guest.vmexe.sha256
        printf '%s %s exe %s vm %s vmexe %s\n' "$n" "$id" "$exe" "$vm" "$(cat /out/$n/guest.vmexe.sha256)" | tee -a /out/ENGINE_IDS
        say "$n built"
    done
    echo "commit $commit" >> /out/ENGINE_IDS
    chown -R "${OUT_UID:-0}:${OUT_GID:-0}" /out
    exit 0
fi
OUT=$REPO/reproduce/out-engine; REV=HEAD
while [ $# -gt 0 ]; do case $1 in --out) OUT=$(realpath -m "$2"); shift 2;; --rev) REV=$2; shift 2;; *) echo "usage: $0 [--out DIR] [--rev REV]" >&2; exit 1;; esac; done
[ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] || echo "note: the checkout has uncommitted changes; the build uses the commit ($REV, git archive)" >&2
# the archive is made HERE, not in the container: a worktree's .git points outside the mounted checkout
COMMIT=$(git -C "$REPO" rev-parse --verify "$REV^{commit}")
IN=$(mktemp -d); trap 'rm -rf "$IN"' EXIT
git -C "$REPO" archive --format=tar "$COMMIT" > "$IN/src.tar"
TAG=hazync-engine-repro:$(sha256sum "$REPO/reproduce/engine.Dockerfile" | cut -c1-16)
docker image inspect "$TAG" > /dev/null 2>&1 || docker build -t "$TAG" -f "$REPO/reproduce/engine.Dockerfile" "$REPO/reproduce" >&2
VOL=hazync-engine-cargo
[ "${HAZYNC_ENGINE_FRESH:-0}" = 1 ] && { docker volume rm -f "$VOL" > /dev/null; }
rm -rf "$OUT" && mkdir -p "$OUT"
docker run --rm -e HAZYNC_ENGINE_IN_CONTAINER=1 -e COMMIT="$COMMIT" -e OUT_UID="$(id -u)" -e OUT_GID="$(id -g)" \
    -v "$REPO":/src:ro -v "$IN":/in:ro -v "$OUT":/out -v "$VOL":/root/.cargo/registry \
    "$TAG" bash /src/reproduce/engine-ids.sh
cat "$OUT/ENGINE_IDS"
