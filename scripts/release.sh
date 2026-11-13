#!/usr/bin/env bash
# Cut a Hazync release. ONE COMMAND, from a clean checkout to a signed, verified, `latest` release.
#
#   ./scripts/release.sh v0.17.0
#   ./scripts/release.sh v0.17.0 --dry-run     # preflight + report only, changes nothing
#
# WHY THIS EXISTS. Every piece of a release already existed as its own script; nothing drove them.
# The ORDER, and roughly a dozen traps, lived only in coordinator/deploy/RUNBOOK.md — so cutting a
# release meant reading a runbook correctly while tired, and each trap below is one that was actually
# walked into, in production, at least once:
#
#   * dist/ is not cleaned between releases, so it holds the LAST release's binaries
#   * a stale artifact can be BYTE-IDENTICAL IN SIZE to a correct one (the id is a fixed-width hex
#     literal), so size and mtime are both worthless as staleness signals
#   * release-sign.yml signs WHATEVER IS ATTACHED AT PUBLISH TIME and never picks up late additions
#   * publishing does NOT make a release `latest`, and older `gh` has no --latest at all
#   * the /latest/download/ CDN serves the PREVIOUS binary for minutes after the pointer is fixed
#   * a CUDA host cannot self-report its id on a box with no card, so it needs explicit attestation
#
# Every one of those is now a step below that fails loudly instead of a paragraph someone must recall.
#
# WHAT IT SHIPS (since v0.23.0, the libzync release): the libzync host for the published mainnet image
# (reproduce/LIBZYNC_IDS) for Linux CPU and CUDA and for Windows CUDA, the libzync verifier
# (swiftsync-verify) as CLI, WebAssembly and its JavaScript glue, and the proof-party scripts. The exact
# list is scripts/release-assets.txt, which release-sign.yml reads too. The CORE design's last release
# is v0.22.1, whose assets stay where they are.
#
# RESUMABLE. Each phase checks whether its work is already done and skips it. A failure at phase 5
# does not rebuild the guest; just run the same command again.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

TAG="${1:-}"
DRY=0
VERIFY_ONLY=0
[ "${2:-}" = "--dry-run" ] && DRY=1
# Resume at step 6 for a release that is already tagged and published but not yet signed — the state
# a cancelled signing job leaves behind. Re-running from the top there is impossible by design:
# preflight refuses because the tag exists. Without this the only route back was reading the script.
[ "${2:-}" = "--verify-only" ] && VERIFY_ONLY=1

REPO_SLUG="${REPO_SLUG:-hazync/hazync}"
DIST="${DIST:-dist}"
# Every asset this script uploads, and the ones release-sign.yml adds, from ONE list both read
# (scripts/release-assets.txt). The aarch64 verifier is NOT uploaded here: CI builds it from the tag and
# attaches it during signing (#85/#90), so it cannot exist before publish.
BINS=$(awk '$1=="release" {print $2}' scripts/release-assets.txt | xargs)
SIGNED_EXTRA=$(awk '$1=="sign" {print $2}' scripts/release-assets.txt | xargs)
WIN=hazync-host-x86_64-windows-cuda.zip
# The published mainnet guest ELF, staged by prover/build-release.sh. The Windows host is checked
# against it (scripts/windows-host.sh), and it is where a host build finds it again on a resumed run.
LZ_ELF=dist/.libzync-guest/mainnet.bin

die()  { echo; echo "FAIL  $*" >&2; exit 1; }
ok()   { echo "  ok   $*"; }
step() { echo; echo "=== $* ==="; }
run()  { [ "$DRY" = 1 ] && { echo "  (dry-run, would run) $*"; return 0; }; "$@"; }

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$ ]] \
    || die "usage: $0 vMAJOR.MINOR.PATCH [--dry-run|--verify-only]   (release-sign.yml refuses anything else)"
[ "$(echo $BINS | wc -w)" -ge 9 ] || die "scripts/release-assets.txt lists only $(echo $BINS | wc -w) release asset(s)"

# The id this release ships: the PUBLISHED libzync mainnet image, from its record. reproduce/METHOD_ID
# names the same id since #676; if they disagree the repo contradicts itself, and nothing is shipped.
CANON=$(awk '$1=="libzync-product" && $2=="mainnet" {print $4}' reproduce/LIBZYNC_IDS)
CANON_SHA=$(awk '$1=="libzync-product" && $2=="mainnet" {print $6}' reproduce/LIBZYNC_IDS)
[[ "$CANON" =~ ^[0-9a-f]{64}$ && "$CANON_SHA" =~ ^[0-9a-f]{64}$ ]] || die "no libzync-product mainnet line in reproduce/LIBZYNC_IDS"
_mid=$(grep -vE '^[[:space:]]*#' reproduce/METHOD_ID | grep -oE '[0-9a-f]{64}' | head -1)
[ "$_mid" = "$CANON" ] || die "reproduce/METHOD_ID (${_mid:0:8}) is not the published libzync mainnet id (${CANON:0:8})"
echo "releasing $TAG at guest ${CANON:0:8}…"
[ "$DRY" = 1 ] && echo "(DRY RUN — nothing will be built, tagged, or published)"

if [ "$VERIFY_ONLY" = 1 ]; then
    echo "(VERIFY ONLY — steps 1-5 skipped; $TAG must already be tagged and published)"
    gh release view "$TAG" >/dev/null 2>&1 \
        || die "$TAG is not published — --verify-only resumes an existing release, it does not create one"
fi

if [ "$VERIFY_ONLY" = 0 ]; then

# ---- 1. preflight ------------------------------------------------------------------------------
# All of these are cheap and all of them have bitten. Do them before a 60-minute build, not after.
step "1. preflight"

[ -z "$(git status --porcelain)" ] || die "working tree is dirty — commit or stash first.
       The container build mounts this tree LIVE and compiles whatever is in it at the moment each
       crate is built, so uncommitted changes end up in the binary with nothing recording it."
ok "working tree clean"

BR=$(git rev-parse --abbrev-ref HEAD)
HEAD_SHA=$(git rev-parse HEAD)
git fetch -q origin main 2>/dev/null
[ "$(git rev-parse origin/main)" = "$HEAD_SHA" ] \
    || die "HEAD ($BR, ${HEAD_SHA:0:8}) is not origin/main ($(git rev-parse --short origin/main)).
       Release from main: the signing workflow checks out THE TAG, and CI results you are trusting
       are for main."
ok "HEAD == origin/main (${HEAD_SHA:0:8})"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists locally"
git ls-remote --exit-code --tags origin "$TAG" >/dev/null 2>&1 && die "tag $TAG already exists on origin"
ok "tag $TAG is free"

./scripts/check-versions.sh >/dev/null 2>&1 || { ./scripts/check-versions.sh; die "check-versions failed"; }
ok "check-versions: every documented id is canonical"

if command -v gh >/dev/null; then
    # Count TOTAL runs as well as non-green ones. Counting only failures makes "no runs at all" and
    # "everything passed" the same answer — both are zero — so pushing and releasing inside the minute
    # before Actions queues anything printed "CI green" having checked nothing (audit #6, F-1). This
    # script exists so a tired operator does not have to remember traps; a gate that cannot fail is
    # the exact trap it is here to mechanise away.
    #
    # Deliberately still ADVISORY when gh cannot answer: an unreachable API must not block a release,
    # only an answered "no" or an unanswerable "nothing ran" should.
    # Count PASSES and FAILURES separately, and treat `cancelled` as neither.
    #
    # "Zero non-green" was the obvious reading and it is wrong, measured the first time it ran: the
    # concurrency group cancels a superseded push run, so a commit that was pushed and then also
    # dispatched carries one `success` and one `cancelled` forever. Counting cancelled as non-green
    # made that commit permanently unreleasable, with the failure text claiming CI had not passed when
    # it had. A cancelled run is a run that was never allowed to have an opinion.
    #
    # Requiring a PASS rather than merely "no failures" keeps the audit-#6 property that made this
    # check exist: zero runs and all-green must not look the same.
    Q() { gh run list --limit 40 --json headSha,conclusion \
          --jq "[.[]|select(.headSha==\"$HEAD_SHA\")|select($1)]|length" 2>/dev/null || echo "?"; }
    # (A `deployed-verifier` exemption lived here: that job tested the CORE browser verifier on the
    # live site and was removed with it in #676, so every failed run counts again.)
    PASS=$(Q '.conclusion=="success"')
    BAD=$(Q '.conclusion|IN("failure","timed_out","startup_failure","action_required")')
    [ "$PASS" = "?" ] && BAD="?"
    if [ "$PASS" = "?" ] || [ "$BAD" = "?" ]; then
        echo "  --   could not read CI status (continuing)"
    elif [ "$BAD" != "0" ]; then
        die "$BAD failed CI run(s) on ${HEAD_SHA:0:8} — fix or re-run before releasing"
    elif [ "$PASS" = "0" ]; then
        die "no PASSING CI run for ${HEAD_SHA:0:8} — nothing has tested this commit.
       (Cancelled or queued runs do not count. If Actions is degraded, that is exactly when this
       matters: check https://www.githubstatus.com before overriding.)"
    else
        ok "CI green on ${HEAD_SHA:0:8} ($PASS passing run(s))"
    fi
fi

command -v docker >/dev/null || die "docker is required for the canonical container build"
command -v gh >/dev/null || die "gh is required: the Windows host is fetched from a windows-prove.yml run"
command -v node >/dev/null || die "node is required: step 4 runs the WebAssembly verifier on a real proof"
command -v python3 >/dev/null || die "python3 is required (byte checks, the Windows zip)"
FREE_GB=$(df -BG --output=avail . | tail -1 | tr -dc '0-9')
[ "${FREE_GB:-0}" -ge 25 ] || die "only ${FREE_GB}GB free; a CUDA build needs 25GB.
       It does NOT report as a disk error — nvcc segfaults (exit status: 139) and reads as a
       toolchain bug. Reclaim first: docker system prune -af"
ok "disk ${FREE_GB}GB free (>=25 needed for cuda)"

# ---- 2. build ----------------------------------------------------------------------------------
# Skipped per-artifact when the staged file already carries the canonical id, so a resumed run after
# a late failure does not rebuild the guest.
step "2. build host binaries (canonical container)"
mkdir -p "$DIST"

# The guest id is NECESSARY but NOT SUFFICIENT evidence that a staged host is current, and this was
# walked into: v0.18.0 changed only the HOST (a new dump-snapshot command) and the verifier FFI, so
# METHOD_ID did not move — and the previous release's binary answered `method-id` with the canonical
# id while containing none of the new code. It would have shipped v0.17.0 under a new tag.
#
# Every earlier release was a re-baseline, where a stale artifact failed the id check for free. That
# made the id look like a staleness signal. It is not one: it is a signal about the GUEST, and a host
# release can change everything else while leaving it untouched.
#
# So staleness is judged against the SOURCE an artifact was built from as well. HEAD is used rather
# than a hash of the host's own files: it over-rebuilds on a docs-only commit, which costs one
# container build a release, and it cannot MISS a change, which is the failure that matters. Phase 1
# already pins HEAD to origin/main, so this is a stable identity.
SRC_REV="$(git rev-parse HEAD 2>/dev/null)"

host_is_current() {  # $1 = path; $2 = asset name; hosts store the id as [u32;8], so ASK, do not grep
    [ -f "$1" ] || return 1
    [ -n "$SRC_REV" ] || return 1
    # The load-bearing staleness check. Everything below only asks WHICH GUEST the binary carries.
    [ "$(cat "$DIST/.built-from-$2" 2>/dev/null)" = "$SRC_REV" ] || return 1

    local id
    id="$("$1" method-id 2>/dev/null | grep -oE '[0-9a-f]{64}' | head -1)"
    if [ -n "$id" ]; then
        [ "$id" = "$CANON" ]
    else
        # A CUDA host cannot run without libcuda.so.1, so on a GPU-less release box "ask the binary"
        # is unanswerable — and an unanswerable question was being read as "stale". The CUDA host
        # therefore rebuilt on EVERY release here, tens of minutes each time, with a correct binary
        # already staged. Fall back to reading the id out of its BYTES, exactly as step 4 does and
        # with the same strength: step 4 already trusts that check to gate publication.
        ./scripts/embeds-method-id.sh "$1" "$CANON"
    fi
}
for mode in cpu cuda; do
    asset="hazync-host-x86_64-linux-gnu"; [ "$mode" = cuda ] && asset="$asset-cuda"
    if host_is_current "$DIST/$asset" "$asset"; then
        ok "$asset already canonical and built from this source — skipping rebuild"
    else
        echo "  building $mode (the host around the published guest ELF; cuda kernels take tens of minutes)…"
        # SKIP_GROTH16: a runtime rzup component the host does not link against — pure download cost.
        # RZUP_TIMEOUT: the default is far too short for the 488MB toolchain on a domestic link.
        # OUT: into $DIST. Without it both scripts wrote to ./dist whatever DIST said, so DIST=elsewhere
        # gated and uploaded a directory nothing had built into.
        run env SKIP_GROTH16=1 RZUP_TIMEOUT="${RZUP_TIMEOUT:-7200}" ${IMAGE:+IMAGE="$IMAGE"} OUT="$(realpath -m "$DIST")" \
            ./prover/build-release.sh "$mode" || die "$mode build failed"
        # Record what it was built from, so the next release can tell a fresh artifact from a stale
        # one that merely embeds the right guest. Written only after the build succeeds.
        [ "$DRY" = 1 ] || echo "$SRC_REV" > "$DIST/.built-from-$asset"
    fi
done

step "2b. the Windows CUDA host (built by windows-prove.yml on GitHub's Windows runners)"
# Not built here: it needs MSVC and the Windows CUDA toolkit. scripts/windows-host.sh takes the
# `hazync-host-windows-x86_64-cuda` artifact of a windows-prove run OF THIS COMMIT whose build-cuda job
# passed, checks host.exe embeds the published guest and id, and zips it with cudart64_12.dll.
# Current = the zip was fetched for HEAD (its .run file says so) and still passes the check.
if [ ! -f "$LZ_ELF" ] && [ -n "${HAZYNC_GUEST_ELF:-}" ]; then
    mkdir -p "$(dirname "$LZ_ELF")" && cp "$HAZYNC_GUEST_ELF" "$LZ_ELF"
fi
[ "$(sha256sum "$LZ_ELF" 2>/dev/null | cut -c1-64)" = "$CANON_SHA" ] || [ "$DRY" = 1 ] \
    || die "$LZ_ELF is not the published mainnet ELF (prover/build-release.sh stages it; or set HAZYNC_GUEST_ELF)"
if [ "$(cut -d' ' -f2 "$DIST/.$WIN.run" 2>/dev/null)" = "$SRC_REV" ] \
   && ./scripts/windows-host.sh check "$DIST/$WIN" "$LZ_ELF" >/dev/null 2>&1; then
    ok "$WIN already fetched for this commit (windows-prove run $(cut -d' ' -f1 "$DIST/.$WIN.run"))"
else
    run ./scripts/windows-host.sh fetch "$SRC_REV" "$DIST/$WIN" "$LZ_ELF" || die "could not obtain the Windows host"
fi

step "3. package worker, wasm + glue and the x86_64 verifier"
# #247: package-release.sh SKIPS the wasm (stderr warning, exit 0) when this box has no wasm32
# target -- which is every release from hazync-coord, which has no host Rust at all. This used to print
# "ok worker + wasm packaged" regardless. Report what actually happened; step 4 still gates the file.
PKG_ERR=$( { run env TAG="$TAG" OUT="$(realpath -m "$DIST")" ./scripts/package-release.sh >/dev/null; } 2>&1 ) || { printf '%s\n' "$PKG_ERR" >&2; die "package-release failed"; }
[ -n "$PKG_ERR" ] && printf '%s\n' "$PKG_ERR" >&2
if printf '%s' "$PKG_ERR" | grep -q "skipping hazync-verify.wasm"; then
    ok "worker packaged"
    if [ -f "$DIST/hazync-verify.wasm" ]; then
        echo "  !    wasm NOT built here -- the staged $DIST/hazync-verify.wasm ($(sha256sum "$DIST/hazync-verify.wasm" | cut -c1-16)) is from elsewhere; step 4 checks its guest"
    else
        echo "  !    wasm NOT built here and none is staged -- build it where wasm32 exists (./swiftsync-verify-wasm/build.sh)"
        echo "       and copy it to $DIST/hazync-verify.wasm, or: rustup target add wasm32-unknown-unknown"
    fi
else
    ok "worker + wasm packaged"
fi
# The libzync verifier CLI (swiftsync-verify; its other binaries, zync-path and zync-serve, are not
# shipped). Built EVERY time rather than when the staged copy lacks the id: an id check cannot see a
# verifier change that leaves the image alone, which is the host staleness trap of v0.18.0 (step 2)
# in another binary. An unchanged tree makes this a no-op.
run cargo build -q --release --locked --manifest-path swiftsync-verify/Cargo.toml --bin swiftsync-verify || die "verifier build failed"
run install -m 0755 swiftsync-verify/target/release/swiftsync-verify "$DIST/hazync-verify-x86_64-linux-gnu"
ok "x86_64 verifier staged"

# ---- 4. gate -----------------------------------------------------------------------------------
# The last point at which a wrong artifact is free to fix. After this it is public and signed.
step "4. verify every staged artifact belongs to guest ${CANON:0:8}"
for b in $BINS; do [ -s "$DIST/$b" ] || die "$DIST/$b is missing or empty — nothing to publish"; done
# EXACTLY the release set, nothing else: dist/ is not cleaned between releases, and check-dist.sh
# judges every file in it. A stray file is either a stale artifact or an asset this list forgot.
extra=""
for f in "$DIST"/*; do
    [ -f "$f" ] || continue
    case " $BINS " in *" $(basename "$f") "*) ;; *) extra="$extra $(basename "$f")";; esac
done
[ -z "$extra" ] || die "$DIST holds files that are not release assets:$extra
       Move them out (or add them to scripts/release-assets.txt if they should ship)."
ok "$DIST holds exactly the $(echo $BINS | wc -w) release assets"

# A CUDA host cannot run without libcuda.so.1. Rather than force a GPU box, read the id out of the
# bytes: the host stores it as [u32;8] LITTLE-ENDIAN, which puts the raw 32 bytes on disk in natural
# order. NB `grep -P '\x..'` silently finds nothing here — it is a check that cannot pass. Use python.
CU="$DIST/hazync-host-x86_64-linux-gnu-cuda"
# ⛔ THE CONDITION IS "CAN THIS BINARY RUN HERE", NOT "IS IT STALE" (#275). This used to sit under
# `! host_is_current "$CU" …`, which is TRUE for exactly the artifact that needs the fallback: a
# freshly built CUDA host matches .built-from, and host_is_current's own byte fallback then finds the
# id — so it reports current, the attestation is skipped, and check-dist.sh fails with "belongs to a
# different guest" over a perfectly good binary. v0.21.1 and v0.21.2 were both published only because
# the operator passed HAZYNC_ATTEST_… by hand. The dead branch could not be seen from either end:
# step 4 looked like it had a fallback, and the message named the wrong problem.
#
# Build the variable name EXACTLY as check-dist.sh does. `tr -c 'A-Za-z0-9' '_'` looks equivalent and
# is not: it rewrites echo's trailing NEWLINE to an underscore too, yielding HAZYNC_ATTEST_..._cuda_ —
# a variable check-dist.sh never reads, so the attestation would be silently ignored and the gate
# would fail with the artifact sitting right there.
_cu=hazync-host-x86_64-linux-gnu-cuda
_att="HAZYNC_ATTEST_${_cu//[^A-Za-z0-9]/_}"
# A host that cannot run here (no NVIDIA driver on the release box) is checked from its BYTES, as before v0.23.0
# (operator decision 2026-11-09: no GPU rental per release): `method-id` prints a constant compiled into the binary
# and never touches CUDA, so the id found in the bytes is the id it would print. An operator who did run it on a card
# can pass the measured id instead, and that is preferred.
if timeout 120 "$CU" method-id >/dev/null 2>&1; then
    :   # it runs here — check-dist.sh will ask it directly, which is the strongest evidence there is
elif [ -n "${!_att:-}" ]; then
    # An operator-supplied id is STRONGER than anything readable from the bytes: it was measured by
    # running this binary on a real GPU. Never overwrite it — including when it is wrong, because
    # check-dist.sh refusing a bad hand-typed attestation is the point of passing one.
    ok "cuda host: cannot execute here; using the attestation you supplied (${!_att:0:8}…)"
    # ...and the bytes must agree with it: an attestation typed for a DIFFERENT file than the one
    # staged would otherwise pass.
    ./scripts/embeds-method-id.sh "$CU" "$CANON" \
        || die "cuda host: attested, but the staged file does not carry ${CANON:0:8} in its bytes exactly once —
       the attestation is for a different binary than the one about to ship"
elif ./scripts/embeds-method-id.sh "$CU" "$CANON"; then
    ok "cuda host: cannot execute here, but embeds ${CANON:0:8} in its bytes"
    echo "  !    that is a BYTE check, not a run. Nothing here has proved this binary works on a GPU —"
    echo "       smoke it on a real card and pass $_att=<measured id> to gate on that instead."
    export "$_att=$CANON"
    # ⛔ Mark the PROVENANCE so check-dist.sh does not report this as an operator attestation
    # (hazync#579). Without it the log reads "attested canonical from a capable host" on a box with
    # no card — the strongest wording available and the least true one.
    export "${_att}_SOURCE=bytes"
else
    # Say which of the two failures this is. "belongs to a different guest" is what check-dist.sh
    # prints next, and it is wrong for a binary that is simply unreadable here.
    echo "  !    cuda host: cannot execute here AND does not carry ${CANON:0:8} in its bytes exactly once."
    echo "       Either it was built from a different guest, or it is not the artifact you think it is."
fi
# The Windows host never runs here; check-dist.sh checks its bytes (the embedded guest's sha256 and the id). An
# operator who ran it may pass HAZYNC_ATTEST_${WIN//[^A-Za-z0-9]/_}=<what host.exe method-id printed>, preferred.
export HAZYNC_GUEST_ELF_STAGED="$LZ_ELF"
./scripts/check-dist.sh "$DIST" || die "a staged artifact belongs to a different guest, or is unattested — do NOT publish"

# An embedded id says which image a verifier is pinned to, not that it works: run the staged verifiers
# (CLI, WASM through its glue, and each host that runs here) on a real receipt and on tampered ones.
./scripts/check-release-verifiers.sh "$DIST" || die "a staged verifier accepted a bad proof or refused a real one — do NOT publish"

echo "  staged assets (sha256):"
( cd "$DIST" && sha256sum $BINS ) | sed 's/^/    /'


if [ "$DRY" = 1 ]; then echo; echo "DRY RUN COMPLETE — preflight and artifacts are good for $TAG."; exit 0; fi

# ---- 5. publish --------------------------------------------------------------------------------
# ⚠ ORDER IS LOAD-BEARING: release-sign.yml signs whatever is attached WHEN THE RELEASE IS PUBLISHED
# and will not pick up anything added afterwards. All assets go up in the create call.
step "5. tag and publish"
NOTES="${NOTES:-}"
[ -n "$NOTES" ] && [ -f "$NOTES" ] || die "set NOTES=<file> to your release notes (a release without
       written notes is not worth cutting — say what moved and what it cost)"

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
    || git tag -a "$TAG" "$HEAD_SHA" -m "$TAG" || die "could not create tag"
git ls-remote --exit-code --tags origin "$TAG" >/dev/null 2>&1 \
    || git push origin "refs/tags/$TAG" || die "could not push tag"
ok "tag $TAG pushed"

if ! gh release view "$TAG" >/dev/null 2>&1; then
    echo "  uploading $(echo $BINS | wc -w) assets (~510MB; slow links take a while)…"
    ( cd "$DIST" && gh release create "$TAG" --title "$TAG" --notes-file "$(cd - >/dev/null; realpath "$NOTES")" $BINS ) \
        || die "release create failed. The tag exists, so fix and re-run: this step is resumable."
fi
# Read back what the release ACTUALLY carries: the exact set, each the size of the staged file. A
# create that uploads nothing, or some, must not be called published — that is how an empty release
# went out once, in another project, with every later check passing.
ASSETS=$(gh release view "$TAG" --json assets --jq '.assets[]|"\(.name) \(.size)"' 2>/dev/null)
for b in $BINS; do
    got=$(printf '%s\n' "$ASSETS" | awk -v n="$b" '$1==n {print $2}')
    [ "$got" = "$(stat -c%s "$DIST/$b")" ] || die "the release carries '$b' at ${got:-no} bytes, staged $(stat -c%s "$DIST/$b")"
done
n_assets=$(printf '%s\n' "$ASSETS" | grep -c . || true)
[ "$n_assets" = "$(echo $BINS | wc -w)" ] || die "the release carries $n_assets assets, want exactly $(echo $BINS | wc -w):
$ASSETS"
ok "release published with exactly the $(echo $BINS | wc -w) staged assets, sizes matching"

fi   # end of the build-and-publish half, skipped by --verify-only

# ---- 6. post-publish verification --------------------------------------------------------------
# Everything below answers "what does a DOWNLOADER actually get", which is the only question that
# matters and the one that has been wrong before while every local check passed.

# Die at the one point where the release is HALF DONE — tag pushed, assets public, nothing signed —
# and say exactly how to finish it. Re-running this script from the top is the wrong move there: the
# tag already exists, so preflight refuses, and the operator is left guessing.
recover_unsigned() {
    echo >&2
    echo "FAIL  Sign release did not succeed for $TAG ($1)" >&2
    echo "      The tag is pushed and the assets are PUBLIC, but there is no SHA256SUMS.txt." >&2
    echo "      Anyone downloading now gets unsigned binaries, so do not leave it here." >&2
    echo >&2
    echo "      Check whether this is us or GitHub:" >&2
    echo "          curl -s https://www.githubstatus.com/api/v2/summary.json | grep -o '\"Actions\"[^}]*'" >&2
    echo "      An outage cancelled this job twice for v0.18.2; it normally takes about a minute." >&2
    echo >&2
    if [ -n "${SIGN_RUN:-}" ]; then
        echo "      Re-run signing (idempotent — uploads use --clobber):" >&2
        echo "          gh run rerun $SIGN_RUN && gh run watch $SIGN_RUN" >&2
    else
        echo "      No run was created. Re-trigger by re-pushing the tag:" >&2
        echo "          git push --force origin $TAG" >&2
    fi
    echo >&2
    echo "      Meanwhile, keep 'latest' pointing at the last SIGNED release:" >&2
    echo "          gh api -X PATCH repos/$REPO_SLUG/releases/\$(gh api repos/$REPO_SLUG/releases/tags/$TAG --jq .id) -F prerelease=true" >&2
    echo "      (\`gh release edit --prerelease\` silently does nothing here — use the API and re-read.)" >&2
    echo >&2
    echo "      Then re-verify from step 7 onward:  ./scripts/release.sh $TAG --verify-only" >&2
    exit 1
}

step "6. wait for signing"
# Wait for THIS TAG's signing run, not simply the newest one with the right name.
#
# Selecting `[select(.name=="Sign release")][0]` matched the PREVIOUS release's run, which had long
# since succeeded, and reported "completed" before this tag's run had even been created. v0.18.1 was
# declared signed while its signing workflow was still queued; step 7 then failed reporting a missing
# binary, when the manifest simply did not exist yet. Two things were wrong at once, and the visible
# one was the wrong one.
#
# `headBranch` carries the tag for a tag-triggered run, so filtering on it makes "the run for this
# release" exact rather than approximate. A missing run is NOT success: it means the workflow has not
# appeared yet, so keep waiting rather than falling through.
#
# "Assets are public but unsigned" is REACHABLE and not always our fault — a GitHub Actions outage
# cancelled this job twice for v0.18.2, a job that takes about a minute normally. So this must not
# just report the state; it must say how to get out of it, because at that point the tag is pushed,
# the assets are up, and re-running the release script from the top is the wrong move.
SIGN_RUN=""
SIGNED=0
for _ in $(seq 1 40); do
    R=$(gh run list --limit 20 --json name,status,conclusion,headBranch,databaseId \
        --jq "[.[]|select(.name==\"Sign release\" and .headBranch==\"$TAG\")][0]|\"\(.status)|\(.conclusion)|\(.databaseId)\"" 2>/dev/null)
    S="${R%|*}"; [ "${R##*|}" != null ] && SIGN_RUN="${R##*|}"
    case "$S" in
        completed\|success) ok "Sign release completed for $TAG"; SIGNED=1; break;;
        completed\|*) recover_unsigned "$S";;
        ""|null*) ;;                # run not created yet — keep waiting
    esac
    sleep 30
done
# Falling out of the loop is NOT success. Before, 20 minutes of "queued" dropped through to step 7,
# which then failed reporting a missing binary — describing the symptom and hiding the cause.
[ "$SIGNED" = 1 ] || recover_unsigned "timed out waiting"

step "7. the signed manifest must cover EVERY binary"
# A manifest covering only some artifacts is worse than none: it looks complete.
M=$(mktemp); A=$(mktemp)
curl -sL -o "$M" "https://github.com/$REPO_SLUG/releases/download/$TAG/SHA256SUMS.txt"
curl -sL -o "$A" "https://github.com/$REPO_SLUG/releases/download/$TAG/SHA256SUMS.txt.asc"
for b in $BINS hazync-verify-aarch64; do
    grep -q " $b\$" "$M" || die "$b is NOT in the signed manifest"
done
ok "manifest covers all $(( $(echo $BINS | wc -w) + 1 )) binaries"
# ...and signs the bytes that were staged and checked here, not merely their names.
if [ "$VERIFY_ONLY" = 0 ]; then
    for b in $BINS; do
        [ "$(awk -v n="$b" '$2==n {print $1}' "$M")" = "$(sha256sum "$DIST/$b" | cut -c1-64)" ] \
            || die "the manifest's sha256 for $b is not the staged file's"
    done
    ok "manifest hashes equal the staged files'"
fi
# The final release: EXACTLY scripts/release-assets.txt, every line, nothing more.
FINAL=$(gh release view "$TAG" --json assets --jq '.assets[].name' 2>/dev/null | sort)
WANT=$(printf '%s\n' $BINS $SIGNED_EXTRA | sort)
[ "$FINAL" = "$WANT" ] || die "the release's assets are not exactly scripts/release-assets.txt:
       on the release: $(echo $FINAL)
       wanted:         $(echo $WANT)"
ok "release carries exactly the $(echo $WANT | wc -w) assets of scripts/release-assets.txt"
gpg --verify "$A" "$M" 2>&1 | grep -q "Good signature" \
    && ok "PGP: good signature" \
    || echo "  --   could not verify PGP locally (is the maintainer key imported?)"

step "8. the latest pointer — checked TWICE, by id and not by size"
# Failure 1 (v0.15.0): a single PATCH setting draft+make_latest published but left `latest` on the
# PREVIOUS tag. Set it explicitly and then read it back rather than assuming it took.
gh api -X PATCH "repos/$REPO_SLUG/releases/$(gh api "repos/$REPO_SLUG/releases/tags/$TAG" --jq .id)" \
    -F make_latest=true >/dev/null 2>&1
GOT=$(gh api "repos/$REPO_SLUG/releases/latest" --jq .tag_name 2>/dev/null)
[ "$GOT" = "$TAG" ] || die "the latest pointer is '$GOT', not $TAG"
ok "API: /releases/latest resolves to $TAG"

# Failure 2 (v0.15.0): the CDN served the OLD binary for minutes AFTER the pointer was fixed. The
# VERSIONED url was correct throughout, so checking that proves nothing about what a user receives.
V=$(mktemp)
for i in $(seq 1 10); do
    curl -sL -H 'Cache-Control: no-cache' -o "$V" \
        "https://github.com/$REPO_SLUG/releases/latest/download/hazync-verify-x86_64-linux-gnu"
    if grep -aq "${CANON:0:8}" "$V"; then ok "CDN: /latest/download/ serves guest ${CANON:0:8}"; break; fi
    [ "$i" = 10 ] && die "after 10 tries /latest/download/ still does not serve ${CANON:0:8}.
       Size is NOT the signal here — a stale verifier is byte-identical in size to a correct one."
    echo "  (CDN still serving a stale copy; retrying in 30s — this took minutes on v0.15.0)"; sleep 30
done
rm -f "$M" "$A" "$V"

echo; echo "DONE — $TAG is published, signed, verified, and latest."
echo "https://github.com/$REPO_SLUG/releases/tag/$TAG"
echo
echo "NOT done automatically, because it needs a machine with the hardware:"
echo "  * prove on the CUDA hosts where a real GPU exists (method-id was attested before publishing):"
echo "        host libzync-prove-range 1 1 testdata/libzync/mainnet genesis out && host libzync-verify out/block_1.rcpt"
echo "  * repoint any running provers at this guest — run-workers.sh only checks its id at STARTUP,"
echo "    so loops started before a re-baseline keep proving into rejections forever (#99)"
echo "  * ⛔ CUT OVER THE BROWSER VERIFIER if hazync-verify.wasm changed: serve this release's"
echo "    hazync-verify.wasm AND hazync-verify.js together (the glue's API is the libzync one, not the"
echo "    CORE module's), and update the page's pinned sha256 of the wasm."
echo "    ⚠ scripts/check-deployed-verifier.sh still drives the CORE module (history/verifier-wasm/), so it"
echo "    cannot confirm this cutover until it is rewritten for the libzync glue."
