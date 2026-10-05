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
say "verifying the embedded guest (this is what makes proofs acceptable)…"
if ! python3 "$root/scripts/embedded_guest_sha.py" "$exe" --root "$root" >"$work/verify.txt" 2>&1; then
  echo "⛔ REFUSING TO PUBLISH: the binary does not carry the canonical guest." >&2
  sed 's/^/     /' "$work/verify.txt" >&2
  exit 1
fi
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
An experimental native Windows build of the Hazync prover, published so the Windows GUI
(\`tools/win-gui\`) can download it without a GitHub login.

**What is established**

- The binary carries the canonical guest, \`35e3f55e…\`, verified before publishing.
- \`host.exe method-id\` prints the canonical \`37987b85…\` on real hardware.
- \`host.exe regress\` passes — the full consensus path, natively on Windows.
- The **CPU** build proves a block on Windows.

**What is not**

- Native Windows **CUDA** proving has never completed. It aborts on the first GPU call.
- A CUDA build imports \`nvcuda.dll\` from the NVIDIA driver, so it cannot even start on a
  machine without one. That is not a corrupt download. Use the CPU build there.

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
