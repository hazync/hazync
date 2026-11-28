#!/bin/bash
# Assemble the release artifacts that do NOT need the fixed-path container.
#
# `prover/build-release.sh {cpu|cuda}` builds the host binaries inside ubuntu:22.04 with HOME=/root,
# because the guest image id absorbs $HOME/.cargo paths and a host-built binary would carry a
# non-canonical id. Neither artifact here contains a guest, so neither has that constraint:
#
#   hazync-worker       the Proof Party contributor CLI (hint -> prove -> submit; #37 removed claims)
#   hazync-verify.wasm  the browser verifier (swiftsync-verify-wasm, the libzync verifier)
#   hazync-verify.js    the JavaScript glue a page needs to load that module (raw ABI, no wasm-bindgen)
#
# Both belong on the release and in the signed SHA256SUMS. The worker especially: it holds the
# contributor's ed25519 signing key and decides what is submitted under their name, so shipping it
# unsigned — or, as before, not shipping it at all and telling people to clone — is the worst option
# on the list.
#
# THE ENGINE RELEASE (28.11.2026, engine/RELEASE_TOOLING.md; the user: "full package", every zAPP CLI and wasm
# module ships, in the signed manifest). Also staged here:
#   hazync-zapp-<name>-x86_64-linux-gnu   every zAPP CLI, built like the verifier CLI (build-linux-verifier.sh: the
#                                         hosts' ubuntu:22.04 image, --locked, the glibc floor); HAZYNC_SKIP_CLIS=1 skips
#   hazync-zapp-<name>.wasm / .js         every zAPP's browser module (its build.sh, the pinned container, clean and
#                                         published required) and its glue
#   hazync-keys-<name>.tar.gz             each published image's key files (ENGINE_KEYS_ROOT=<dir with one directory
#                                         per record name>), checked against reproduce/engine-release.tsv, packed
#                                         reproducibly (sorted, owner 0, mtime 0, gzip -n)
#   hazync-engine-x86_64-linux-gnu[-cuda] hzengine, from HZENGINE_CPU / HZENGINE_CUDA (paths): no reproducible release
#                                         build of hzengine exists yet, so this only stages and checks what it is given
# Anything this machine cannot make is SKIPPED with a line on stderr naming what is missing; scripts/release.sh's
# exact-set check then refuses the release, so a skip can never ship a short set.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
OUT="${OUT:-$PWD/dist}"
mkdir -p "$OUT"

# ---- worker -------------------------------------------------------------------------------------
# A single-file Python script with no imports outside the stdlib except `cryptography` (for ed25519),
# so "packaging" is a copy. Renamed to hazync-worker on the release: `hazync` alone is too generic a
# name to drop into someone's PATH, and it must match the `hazync-*` download glob in release-sign.yml.
python3 -c "import ast,sys; ast.parse(open('coordinator/hazync').read())" \
    || { echo "::error::coordinator/hazync does not parse — refusing to ship it"; exit 1; }
cp coordinator/hazync "$OUT/hazync-worker"
chmod +x "$OUT/hazync-worker"

# Stamp the release into the SHIPPED copy (#293). The source keeps VERSION = "dev", because a
# checkout is not a release and saying so is the honest answer; only the artifact claims a version.
# That is what makes drift impossible: nothing to bump by hand, so nothing to forget.
#
# ⛔ ASSERT THE STAMP LANDED. A `sed` that matches nothing exits 0, and the release would then ship a
# binary reporting "dev" for ever — a silent failure of the exact mechanism built to end silent
# failures. TAG is optional: running this script by hand leaves "dev", which is correct.
if [ -n "${TAG:-}" ]; then
    sed -i "s|^VERSION = \"dev\"$|VERSION = \"${TAG#v}\"|" "$OUT/hazync-worker"
    grep -q "^VERSION = \"${TAG#v}\"$" "$OUT/hazync-worker" \
        || { echo "::error::could not stamp VERSION=${TAG#v} into hazync-worker — did the line move?"; exit 1; }
    python3 -c "import ast,sys; ast.parse(open('$OUT/hazync-worker').read())" \
        || { echo "::error::hazync-worker does not parse after stamping"; exit 1; }
fi

# It must run. A syntax check passes on a script whose entry point is broken, and the first thing a
# contributor does is run it — `--help` exercises argument handling and the discovery path without
# touching the network or claiming anything.
"$OUT/hazync-worker" --help >/dev/null 2>&1 \
    || "$OUT/hazync-worker" 2>&1 | head -1 | grep -qi hazync \
    || { echo "::error::hazync-worker does not run"; exit 1; }

# ---- coordinator + fleet launcher -----------------------------------------------------------------
# Everything a person DOWNLOADS AND EXECUTES should be covered by SHA256SUMS.txt.asc, and two things
# were not:
#
#   * `run-workers.sh` — CONTRIBUTING fetched it from raw.githubusercontent, unsigned, on the line
#     immediately after boasting that hazync-worker is "a SIGNED release artifact". HTTPS gives
#     transport security and GitHub's word; it does not give the release signature the same paragraph
#     is selling. This is the script that launches the fleet.
#   * `server.py` — hazync#69 is about third parties running their OWN coordinator, and there was no
#     documented way to get it at all except cloning the repo.
#
# Both are plain scripts, so packaging is a copy — but a copy that lands in the manifest.
python3 -c "import ast,sys; ast.parse(open('coordinator/server.py').read())" \
    || { echo "::error::coordinator/server.py does not parse — refusing to ship it"; exit 1; }
cp coordinator/server.py "$OUT/hazync-coordinator.py"

bash -n coordinator/run-workers.sh \
    || { echo "::error::coordinator/run-workers.sh does not parse — refusing to ship it"; exit 1; }
cp coordinator/run-workers.sh "$OUT/hazync-run-workers.sh"
chmod +x "$OUT/hazync-run-workers.sh"

# Named `hazync-*` deliberately: release-sign.yml downloads the release assets by that glob, so an
# asset outside it is published and then silently left out of the signed manifest — the exact failure
# step 7 of release.sh exists to catch.

# ---- browser verifier ---------------------------------------------------------------------------
# The libzync verifier (swiftsync-verify-wasm): the same library as the CLI, built byte-reproducibly by
# its build.sh. It has a raw ABI, so a page cannot use the .wasm without the glue that frames the
# input and reads the verdict back: ship it beside the module, under the same `hazync-verify` name, so
# it is in the signed manifest too. The glue takes the module's bytes or a fetch() of any URL, so the
# rename costs a page nothing.
cp swiftsync-verify-wasm/swiftsync-verify.js "$OUT/hazync-verify.js"
# build.sh runs in the pinned container reproduce/wasm.Dockerfile: the host needs docker, cargo (>= 1.78, to read the
# crate graph), python3 and git, and NO wasm32 target of its own (the container brings Rust and its target).
if command -v docker >/dev/null && command -v cargo >/dev/null && command -v python3 >/dev/null && command -v git >/dev/null; then
    # a release builds from the committed sources in the PUBLISHED build image only (reproduce/wasm-image.digests);
    # HAZYNC_WASM_ALLOW_LOCAL_IMAGE=1 in the environment is the explicit override for a locally built image
    HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1 ./swiftsync-verify-wasm/build.sh >/dev/null
    cp swiftsync-verify-wasm/target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm "$OUT/hazync-verify.wasm"
else
    echo "! skipping hazync-verify.wasm — swiftsync-verify-wasm/build.sh needs docker, cargo, python3 and git on this machine" >&2
fi

# ---- the engine release: zAPP CLIs and wasm modules, key tarballs, hzengine ------------------------------------
skip() { echo "! skipping $1 — $2" >&2; }
# name : CLI crate dir : CLI bin : wasm dir (- when the zAPP has no wasm module yet) : wasm file : glue
ZAPPS="assume:zapps/assume/cli:zapp-assume:zapps/assume/wasm:zapp_assume_wasm.wasm:zapp-assume.js
deposit:zapps/deposit/cli:zapp-deposit:zapps/deposit/wasm:zapp_deposit_wasm.wasm:zapp-deposit.js
headers:zapps/headers/cli:zapp-headers:zapps/headers/wasm:zapp_headers_wasm.wasm:zapp-headers.js
hints:zapps/hints/cli:zapp-hints:zapps/hints/wasm:zapp_hints_wasm.wasm:zapp-hints.js
liveness:zapps/liveness/cli:zapp-liveness:zapps/liveness/wasm:zapp_liveness_wasm.wasm:zapp-liveness.js
statement:zapps/statement/cli:zapp-statement:zapps/statement/wasm:zapp_statement_wasm.wasm:zapp-statement.js
utreexo:zapps/utreexo/cli:zapp-utreexo:zapps/utreexo/wasm:zapp_utreexo_wasm.wasm:zapp-utreexo.js
zeroconf:zapps/zeroconf/cli:zapp-zeroconf:-:-:-"
HAVE_WASM_TOOLS=0
command -v docker >/dev/null && command -v cargo >/dev/null && command -v python3 >/dev/null && command -v git >/dev/null && HAVE_WASM_TOOLS=1
while IFS=: read -r name crate bin wdir wfile glue; do
    [ -n "$name" ] || continue
    if [ "${HAZYNC_SKIP_CLIS:-0}" = 1 ]; then
        skip "hazync-zapp-$name-x86_64-linux-gnu" "HAZYNC_SKIP_CLIS=1"
    elif command -v docker >/dev/null; then
        ./scripts/build-linux-verifier.sh x86_64 "$OUT/hazync-zapp-$name-x86_64-linux-gnu" "$crate" "$bin" >/dev/null \
            || { echo "::error::the $name zAPP CLI did not build" >&2; exit 1; }
        # it must run: every zAPP CLI answers no arguments with its usage and exit 1 (a FAIL), never a crash (101, a
        # signal) or a missing loader (126/127)
        rc=0; "$OUT/hazync-zapp-$name-x86_64-linux-gnu" >/dev/null 2>&1 < /dev/null || rc=$?
        [ "$rc" = 1 ] || { echo "::error::hazync-zapp-$name-x86_64-linux-gnu does not run (no arguments: exit $rc, want 1)" >&2; exit 1; }
    else
        skip "hazync-zapp-$name-x86_64-linux-gnu" "build-linux-verifier.sh needs docker"
    fi
    if [ "$wdir" = - ]; then
        skip "hazync-zapp-$name.wasm" "the $name zAPP has no wasm module yet (engine/RELEASE_TOOLING.md)"
        continue
    fi
    cp "$wdir/$glue" "$OUT/hazync-zapp-$name.js"
    if [ "$HAVE_WASM_TOOLS" = 1 ]; then
        HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1 "./$wdir/build.sh" >/dev/null \
            || { echo "::error::the $name zAPP wasm module did not build" >&2; exit 1; }
        cp "$wdir/target/wasm32-unknown-unknown/release/$wfile" "$OUT/hazync-zapp-$name.wasm"
    else
        skip "hazync-zapp-$name.wasm" "its build.sh needs docker, cargo, python3 and git on this machine"
    fi
done <<< "$ZAPPS"

# The key tarballs: one per published record (reproduce/engine-release.tsv, written at the Stage 4 freeze).
REC=reproduce/engine-release.tsv
KEYFILES="sdk.pk agg.vk openvm.toml guest.vmexe baseline.json image.json vk.bin"
if [ ! -f "$REC" ]; then
    skip "hazync-keys-*.tar.gz" "$REC does not exist (no engine image is published yet: Stage 4)"
elif [ -z "${ENGINE_KEYS_ROOT:-}" ]; then
    skip "hazync-keys-*.tar.gz" "ENGINE_KEYS_ROOT is not set (a directory holding one key directory per record name)"
else
    while read -r name _net id pin; do
        case "$name" in ''|\#*) continue ;; esac
        d="$ENGINE_KEYS_ROOT/$name"
        for f in $KEYFILES; do [ -s "$d/$f" ] || { echo "::error::$d/$f is missing or empty" >&2; exit 1; }; done
        # the key directory must BE the record: the image id from its image.json, the pin from its vk.bin
        got=$(python3 -c "import json,hashlib,sys;i=json.load(open(sys.argv[1]));print(hashlib.sha256(bytes.fromhex(i['exe']+i['vm'])).hexdigest())" "$d/image.json")
        [ "$got" = "$id" ] || { echo "::error::$d/image.json gives image ${got:0:8}, the $name record is ${id:0:8}" >&2; exit 1; }
        [ "$(sha256sum "$d/vk.bin" | cut -c1-64)" = "$pin" ] || { echo "::error::$d/vk.bin is not the $name record's pinned key" >&2; exit 1; }
        # reproducible: the same files give the same bytes on any machine (sorted, no owner, no time, gzip without a name)
        # shellcheck disable=SC2086  # KEYFILES is a fixed list of plain names
        tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 --format=gnu -cf - -C "$d" $KEYFILES \
            | gzip -9 -n > "$OUT/hazync-keys-$name.tar.gz.tmp" && mv -f "$OUT/hazync-keys-$name.tar.gz.tmp" "$OUT/hazync-keys-$name.tar.gz"
    done < "$REC"
fi

# hzengine: staged from what the operator built (no reproducible release build of it exists yet).
for v in cpu cuda; do
    asset=hazync-engine-x86_64-linux-gnu; [ "$v" = cuda ] && asset="$asset-cuda"
    src=$([ "$v" = cpu ] && echo "${HZENGINE_CPU:-}" || echo "${HZENGINE_CUDA:-}")
    if [ -z "$src" ]; then
        skip "$asset" "no reproducible release build of hzengine yet; set HZENGINE_$(echo $v | tr a-z A-Z)=<binary> to stage one"
        continue
    fi
    [ -x "$src" ] || { echo "::error::$src is not an executable" >&2; exit 1; }
    ./scripts/check-glibc-floor.sh "$src" || { echo "::error::$src needs a newer glibc than the hosts" >&2; exit 1; }
    # the CPU build must run here; a CUDA build needs libcuda, so only its bytes are checked on a GPU-less box
    if [ "$v" = cpu ]; then
        "$src" --help >/dev/null 2>&1 || { echo "::error::$src --help failed" >&2; exit 1; }
    fi
    install -m 0755 "$src" "$OUT/$asset"
done

echo "packaged into $OUT:"
# List everything this script stages, not a subset. A summary that under-reports is how an asset gets
# dropped without anyone noticing — the operator reads the summary, not the source.
for f in hazync-worker hazync-coordinator.py hazync-run-workers.sh hazync-verify.wasm hazync-verify.js \
         hazync-zapp-* hazync-keys-* hazync-engine-x86_64-linux-gnu hazync-engine-x86_64-linux-gnu-cuda; do
    for g in "$OUT"/$f; do
        [ -f "$g" ] && printf '  %-40s %10d bytes  %s\n' "$(basename "$g")" "$(stat -c%s "$g")" "$(sha256sum "$g" | cut -c1-16)"
    done
done
echo
echo "Upload these to the release BEFORE publishing — release-sign.yml signs whatever is attached"
echo "when the release is published, and will not pick up anything added afterwards."
