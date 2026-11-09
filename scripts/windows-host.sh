#!/usr/bin/env bash
# The Windows CUDA host for a release: fetched from CI, never built here, and checked from its bytes.
#
#   scripts/windows-host.sh fetch <commit-sha> <out.zip> <guest-elf>   # download + check + zip
#   scripts/windows-host.sh check <zip> <guest-elf>                     # check a staged zip
#
# WHERE IT COMES FROM. A Windows CUDA host can only be built on Windows (MSVC + the CUDA toolkit);
# `.github/workflows/windows-prove.yml`'s `build-cuda` job does that on windows-2022, embedding the
# published libzync guest that the same run's `guest` job built in reproduce/libzync.Dockerfile, and
# uploads `host.exe` + `cudart64_12.dll` as the artifact `hazync-host-windows-x86_64-cuda`. `fetch`
# takes that artifact from a run of THE RELEASE COMMIT whose `build-cuda` job succeeded. Dispatch it
# before releasing (artifacts expire after 14 days):
#
#     gh workflow run windows-prove.yml --ref main -f skip_cpu_prove=true
#
# WHAT IS CHECKED, with no GPU and no Windows (a CUDA-linked exe imports nvcuda.dll, so it cannot even
# load without an NVIDIA driver; scripts/embedded_guest_sha.py explains):
#   * the zip holds exactly host.exe and the cudart64_*.dll runtime, flat, nothing else;
#   * host.exe embeds the published mainnet guest byte for byte (sha256 against reproduce/LIBZYNC_IDS,
#     the size taken from <guest-elf>, itself checked against the same pin);
#   * host.exe carries the published image id exactly once ([u32;8] little-endian, as on Linux).
# What it CANNOT check is `host.exe method-id`: scripts/release.sh requires that from an operator who
# ran it on a Windows NVIDIA machine (HAZYNC_ATTEST_hazync_host_x86_64_windows_cuda_zip=<id>).
#
# THE ZIP. host.exe and the DLL keep their own names (Windows finds a DLL beside an exe by name), and
# they sit at the top level so "Extract All" gives a folder the Windows GUI searches for host.exe.
# Written with fixed timestamps and order, so zipping the same artifact twice gives the same bytes.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
REPO_SLUG="${REPO_SLUG:-hazync/hazync}"
ART=hazync-host-windows-x86_64-cuda
die() { echo "FAIL $*" >&2; exit 1; }

LZ_LINE=$(grep '^libzync-product mainnet ' reproduce/LIBZYNC_IDS || true)
WANT_ID=$(echo "$LZ_LINE" | awk '{print $4}')
WANT_SHA=$(echo "$LZ_LINE" | awk '{print $6}')
[[ "$WANT_ID" =~ ^[0-9a-f]{64}$ && "$WANT_SHA" =~ ^[0-9a-f]{64}$ ]] || die "no libzync-product mainnet line in reproduce/LIBZYNC_IDS"

guest_size() {  # the published ELF's size, from a copy whose sha256 is the pin
    [ -f "$1" ] || die "no guest ELF at '$1' (prover/build-release.sh stages it at dist/.libzync-guest/mainnet.bin)"
    [ "$(sha256sum "$1" | cut -c1-64)" = "$WANT_SHA" ] || die "$1 is not the published mainnet ELF"
    stat -c%s "$1"
}

# check_exe <dir holding host.exe and the dll(s)> <guest-elf>
check_dir() {
    local d=$1 size
    size=$(guest_size "$2") || exit 1
    [ -f "$d/host.exe" ] || die "no host.exe"
    ls "$d"/cudart64_*.dll >/dev/null 2>&1 || die "no cudart64_*.dll beside host.exe"
    python3 scripts/embedded_guest_sha.py "$d/host.exe" --pin "$WANT_SHA" --size "$size" > "$d/.guest.txt" 2>&1 \
        || { sed 's/^/       /' "$d/.guest.txt" >&2; die "host.exe does not embed the published mainnet guest"; }
    ./scripts/embeds-method-id.sh "$d/host.exe" "$WANT_ID" \
        || die "host.exe does not carry the published image id ${WANT_ID:0:8} exactly once"
    echo "  ok   host.exe embeds the published guest (${WANT_SHA:0:16}…, $size bytes) and image id ${WANT_ID:0:8} once"
}

case "${1:-}" in
  fetch)
    SHA="${2:?usage: $0 fetch <commit-sha> <out.zip> <guest-elf>}"; OUT="${3:?out.zip}"; ELF="${4:?guest-elf}"
    [[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || die "want a full 40-hex commit sha, got '$SHA'"
    # Newest first. A run counts only when ITS build-cuda job succeeded and the artifact is unexpired;
    # the run's own conclusion is not enough (a failed CPU prove job would hide a good CUDA build, and
    # the artifact is uploaded `if: always()`, so it can exist for a failed build).
    run_id=""
    for r in $(gh run list -R "$REPO_SLUG" --workflow windows-prove.yml --limit 50 --json databaseId,headSha \
                 --jq "[.[]|select(.headSha==\"$SHA\")|.databaseId][]" 2>/dev/null); do
        jc=$(gh api "repos/$REPO_SLUG/actions/runs/$r/jobs" --jq '[.jobs[]|select(.name=="build-cuda")|.conclusion][0]' 2>/dev/null)
        [ "$jc" = success ] || { echo "  --   run $r: build-cuda is '${jc:-absent}', skipped"; continue; }
        live=$(gh api "repos/$REPO_SLUG/actions/runs/$r/artifacts" \
                 --jq "[.artifacts[]|select(.name==\"$ART\" and .expired==false)]|length" 2>/dev/null)
        [ "$live" = 1 ] || { echo "  --   run $r: no unexpired $ART artifact, skipped"; continue; }
        run_id=$r; break
    done
    [ -n "$run_id" ] || die "no windows-prove run of ${SHA:0:8} with a successful build-cuda job and a live $ART artifact.
       Dispatch one on the release commit and wait for build-cuda (about an hour cold):
           gh workflow run windows-prove.yml --ref main -f skip_cpu_prove=true"
    echo "  windows-prove run $run_id (${SHA:0:8}): downloading $ART…"
    W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
    gh run download "$run_id" -R "$REPO_SLUG" -n "$ART" -D "$W/a" || die "could not download $ART from run $run_id"
    # Exactly the two kinds of file the workflow uploads, flat. Anything else is not this artifact.
    extra=$(cd "$W/a" && find . -mindepth 1 \( -type d -o -type f \) ! -name host.exe ! -name 'cudart64_*.dll' -print)
    [ -z "$extra" ] || die "unexpected entries in $ART: $extra"
    check_dir "$W/a" "$ELF"
    python3 - "$W/a" "$OUT.tmp" <<'PY' || die "could not write the zip"
import os, sys, zipfile
src, out = sys.argv[1], sys.argv[2]
names = sorted(n for n in os.listdir(src) if not n.startswith('.'))
with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for n in names:
        i = zipfile.ZipInfo(n, date_time=(1980, 1, 1, 0, 0, 0))
        i.compress_type = zipfile.ZIP_DEFLATED
        i.external_attr = 0o100755 << 16
        with open(os.path.join(src, n), 'rb') as f:
            z.writestr(i, f.read(), compresslevel=9)
print('  zipped: ' + ', '.join(names))
PY
    mv -f "$OUT.tmp" "$OUT"
    # Provenance, beside the zip but hidden from the release (a dot-file is not an asset).
    echo "$run_id $SHA" > "$(dirname "$OUT")/.$(basename "$OUT").run"
    echo "  ok   $(basename "$OUT") from windows-prove run $run_id ($(stat -c%s "$OUT") bytes)"
    ;;
  check)
    Z="${2:?usage: $0 check <zip> <guest-elf>}"; ELF="${3:?guest-elf}"
    [ -f "$Z" ] || die "$Z does not exist"
    W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
    python3 - "$Z" "$W" <<'PY' || die "$Z is not the expected zip (host.exe + cudart64_*.dll, flat)"
import fnmatch, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
names = z.namelist()
ok = (names and 'host.exe' in names and any(fnmatch.fnmatch(n, 'cudart64_*.dll') for n in names)
      and all(n == 'host.exe' or fnmatch.fnmatch(n, 'cudart64_*.dll') for n in names))
if not ok:
    print('  entries: ' + ', '.join(names), file=sys.stderr); sys.exit(1)
z.extractall(sys.argv[2])
PY
    check_dir "$W" "$ELF"
    ;;
  *) echo "usage: $0 fetch <commit-sha> <out.zip> <guest-elf> | check <zip> <guest-elf>" >&2; exit 2 ;;
esac
