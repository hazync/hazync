#!/usr/bin/env bash
# Publish the Windows prover as a release asset, so the GUI can fetch it like everything else.
#
#     ./publish_host_asset.sh            # dry run: show what it WOULD publish
#     ./publish_host_asset.sh --publish  # actually do it
#
# ⛔⛔ WHY THIS IS THE LAST THING BETWEEN THE GUI AND "WORKS OUT OF THE BOX". `hazync-worker` is a
# release asset and downloads with no credentials — measured, HTTP 200 unauthenticated. The Windows
# host binary exists ONLY as a CI artifact, and GitHub refuses to serve build artifacts anonymously
# even for a public repository. So a newcomer cannot get the prover without a GitHub login, and the
# GUI cannot fetch it for them. Publishing it as a release asset fixes that permanently.
#
# ⚠ THIS IS NOT RUN AUTOMATICALLY, DELIBERATELY. Publishing is outward-facing and this project's
# standing rule is that a Hazync release is never cut without going through its contents first. So
# it defaults to a DRY RUN and needs --publish to do anything.
#
# ⛔ AND IT VERIFIES BEFORE IT PUBLISHES. A binary whose embedded guest is not canonical produces
# proofs the coordinator rejects, and publishing one would hand that to every newcomer at once.
#
# ⚠ SINCE v0.23.0 THE WINDOWS CUDA HOST IS A REGULAR RELEASE ASSET (hazync-host-x86_64-windows-cuda.zip,
# fetched and checked by scripts/release.sh via scripts/windows-host.sh). This preview path is only for
# a build BETWEEN releases. It checks the same thing: the published libzync mainnet guest
# (reproduce/LIBZYNC_IDS), so it needs that ELF to know the guest's size:
#
#     HAZYNC_GUEST_ELF=dist/.libzync-guest/mainnet.bin ./publish_host_asset.sh

set -uo pipefail
trap 'echo "ERR at line $LINENO (rc=$?)" >&2' ERR

REPO="${REPO:-hazync/hazync}"
TAG="${TAG:-windows-preview}"
PUBLISH=0
[ "${1:-}" = "--publish" ] && PUBLISH=1

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

say() { printf '  %s\n' "$*"; }

say "repo : $REPO"
say "tag  : $TAG"
say "mode : $([ "$PUBLISH" = 1 ] && echo 'PUBLISH' || echo 'dry run (pass --publish to do it)')"
echo

# ── 1. the newest windows-prove run that actually produced a CUDA binary ─────────────────────────
# ⚠ Not "the newest run": most of them failed, and a failed run can still have a log artifact. The
# one that matters is the newest with the host binary present and unexpired.
say "looking for the newest windows-prove run with a Windows host artifact…"
run_id=""
art_id=""
art_name=""
# ⚠ FIVE runs, not twenty. Each candidate costs an API round trip, and twenty of them took long
# enough to be killed by a timeout during testing. The artifact retention is 14 days, so a
# successful run older than the fifth is almost certainly expired anyway.
for r in $(gh api "repos/$REPO/actions/workflows/windows-prove.yml/runs?per_page=5" \
             --jq '.workflow_runs[]|select(.conclusion=="success")|.id' 2>/dev/null); do
  printf '    checking run %s…\n' "$r"
  line=$(gh api "repos/$REPO/actions/runs/$r/artifacts" \
           --jq '.artifacts[]|select(.expired==false and (.name|test("host-windows")))|"\(.id) \(.name)"' \
           2>/dev/null | head -1)
  if [ -n "$line" ]; then
    run_id="$r"; art_id="${line%% *}"; art_name="${line#* }"
    break
  fi
done
if [ -z "$run_id" ]; then
  echo "⛔ no successful windows-prove run has an unexpired host artifact." >&2
  echo "   Artifacts expire after 14 days. Dispatch windows-prove.yml and run this again." >&2
  exit 1
fi
say "run     $run_id"
say "artifact $art_id  ($art_name)"

# ── 2. fetch and unpack it ───────────────────────────────────────────────────────────────────────
say "downloading…"
gh api "repos/$REPO/actions/artifacts/$art_id/zip" > "$work/a.zip" 2>/dev/null || {
  echo "⛔ could not download the artifact" >&2; exit 1; }
python3 - "$work" <<'PY'
import sys, zipfile
w = sys.argv[1]
z = zipfile.ZipFile(f"{w}/a.zip")
z.extractall(w)
print("  contents: " + ", ".join(f"{i.filename} ({i.file_size:,} B)" for i in z.infolist()))
PY
exe="$work/host.exe"
[ -f "$exe" ] || { echo "⛔ no host.exe inside the artifact" >&2; exit 1; }

# ── 3. ⛔ VERIFY BEFORE PUBLISHING ───────────────────────────────────────────────────────────────
# Against the PUBLISHED libzync mainnet image. (reproduce/GUEST_ELF_SHA256 names the same ELF since
# #676, but embedded_guest_sha.py's default size is the retired CORE guest's, so pass both.)
say "verifying the embedded guest (this is what makes proofs acceptable)…"
lz=$(grep '^libzync-product mainnet ' "$root/reproduce/LIBZYNC_IDS")
want_id=$(echo "$lz" | awk '{print $4}'); want_sha=$(echo "$lz" | awk '{print $6}')
elf="${HAZYNC_GUEST_ELF:-$root/dist/.libzync-guest/mainnet.bin}"
[ "$(sha256sum "$elf" 2>/dev/null | cut -c1-64)" = "$want_sha" ] || {
  echo "⛔ need the published mainnet guest ELF (sha256 $want_sha) for its size: set HAZYNC_GUEST_ELF" >&2; exit 1; }
if ! python3 "$root/scripts/embedded_guest_sha.py" "$exe" --root "$root" --pin "$want_sha" --size "$(stat -c%s "$elf")" \
       >"$work/verify.txt" 2>&1; then
  echo "⛔ REFUSING TO PUBLISH: the binary does not carry the published libzync guest." >&2
  sed 's/^/     /' "$work/verify.txt" >&2
  exit 1
fi
"$root/scripts/embeds-method-id.sh" "$exe" "$want_id" || {
  echo "⛔ REFUSING TO PUBLISH: the binary does not carry the image id ${want_id:0:8} exactly once." >&2; exit 1; }
grep -aE "canonical guest embedded|ESTABLISHED" "$work/verify.txt" | sed 's/^/  /'

# ⚠ Also record WHICH build it is, because a CUDA build cannot start without an NVIDIA driver and
# the release notes must say so or newcomers will report it as a broken download.
kind=$(python3 - "$exe" <<'PY'
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), "."))
sys.path.insert(0, os.environ.get("WINGUI", "."))
import supervisor
i = supervisor.classify_host(sys.argv[1])
print(i["kind"])
PY
) || kind="unknown"
say "build kind: $kind"

sha=$(sha256sum < "$exe" | cut -d' ' -f1)
say "sha256    : $sha"
say "size      : $(stat -c%s "$exe" | sed ':a;s/\B[0-9]\{3\}\>/,&/;ta') bytes"

asset="hazync-host-windows-x86_64-${kind}.exe"
say "would publish as: $asset  on tag $TAG"

if [ "$PUBLISH" != 1 ]; then
  echo
  say "dry run only — nothing was published. Re-run with --publish."
  exit 0
fi

# ── 4. publish ───────────────────────────────────────────────────────────────────────────────────
# ⚠ A PRERELEASE on its own tag, not an asset bolted onto v0.22.1. Attaching a Windows binary to a
# tag that never contained one misrepresents what that release was, and this build is explicitly
# experimental: native Windows CUDA proving has not completed yet.
if ! gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
  say "creating prerelease $TAG…"
  gh release create "$TAG" -R "$REPO" --prerelease \
    --title "Windows prover (preview)" \
    --notes "$(cat <<EOF
An experimental native Windows build of the Hazync prover (the libzync host), published so the
Windows GUI (\`tools/win-gui\`) can download it without a GitHub login. Released builds ship as
\`hazync-host-x86_64-windows-cuda.zip\` on the regular releases.

**What is established**

- The binary embeds the published libzync mainnet guest (\`${want_sha:0:8}…\`) and its image id
  \`${want_id:0:8}…\`, both checked from its bytes before publishing.

**What is not**

- Its own \`host.exe method-id\` was not run by this script (it cannot start without an NVIDIA
  driver). Run it on your machine: it must print \`${want_id:0:8}…\`.
- A CUDA build imports \`nvcuda.dll\` from the NVIDIA driver, so it cannot even start on a
  machine without one. That is not a corrupt download.

sha256 \`$sha\`
EOF
)" || { echo "⛔ could not create the release" >&2; exit 1; }
fi

cp "$exe" "$work/$asset"
say "uploading $asset…"
gh release upload "$TAG" "$work/$asset" -R "$REPO" --clobber || {
  echo "⛔ upload failed" >&2; exit 1; }

echo
say "✅ published:"
say "   https://github.com/$REPO/releases/download/$TAG/$asset"
say "The GUI can now fetch the prover itself — no login needed."
