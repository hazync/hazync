#!/usr/bin/env bash
# The Windows hosts for a release: fetched from CI, never built here, and checked from its bytes.
#
#   scripts/windows-host.sh fetch <commit-sha> <out.zip> <guest-elf>   # download + check + zip
#   scripts/windows-host.sh check <zip> <guest-elf>                     # check a staged zip
#
# TWO KINDS, told apart by the zip's name (16.11, owner: ship the CPU prover too, so a machine with no usable NVIDIA
# card can still prove):
#   *-cuda.zip  host.exe + cudart64_*.dll, from windows-prove's `build-cuda` job (artifact hazync-host-windows-x86_64-cuda)
#   *-cpu.zip   host.exe alone, from windows-prove's `prove` job (artifact hazync-host-windows-x86_64-cpu). That job also
#               RAN it on Windows: `method-id` against the pin, and a real prove of mainnet block 1, verified. Its
#               host.exe must not import nvcuda.dll (a CUDA build cannot load on a machine without the NVIDIA driver,
#               which is exactly the machine the CPU prover is for).
#
# WHERE IT COMES FROM. A Windows CUDA host can only be built on Windows (MSVC + the CUDA toolkit);
# `.github/workflows/windows-prove.yml`'s `build-cuda` job does that on windows-2022, embedding the
# published libzync guest that the same run's `guest` job built in reproduce/libzync.Dockerfile, and
# uploads `host.exe` + `cudart64_12.dll` as the artifact `hazync-host-windows-x86_64-cuda`. `fetch`
# takes that artifact from a run of THE RELEASE COMMIT whose `build-cuda` job succeeded. Dispatch it
# before releasing (artifacts expire after 14 days):
#
#     gh workflow run windows-prove.yml --ref main -f release_build=true
#
# WHAT IS CHECKED, with no GPU and no Windows (a CUDA-linked exe imports nvcuda.dll, so it cannot even
# load without an NVIDIA driver; scripts/embedded_guest_sha.py explains):
#   * the zip holds exactly host.exe and the cudart64_*.dll runtime, flat, nothing else;
#   * host.exe embeds the published mainnet guest byte for byte (sha256 against reproduce/LIBZYNC_IDS,
#     the size taken from <guest-elf>, itself checked against the same pin);
#   * host.exe carries the published image id ([u32;8] little-endian, as on Linux; MSVC keeps 2 copies).
# What it CANNOT check is `host.exe method-id`: scripts/release.sh requires that from an operator who
# ran it on a Windows NVIDIA machine (HAZYNC_ATTEST_hazync_host_x86_64_windows_cuda_zip=<id>).
#
# THE ZIP. host.exe and the DLL keep their own names (Windows finds a DLL beside an exe by name), and
# they sit at the top level so "Extract All" gives a folder the Windows GUI searches for host.exe.
# Written with fixed timestamps and order, so zipping the same artifact twice gives the same bytes.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
REPO_SLUG="${REPO_SLUG:-hazync/hazync}"
die() { echo "FAIL $*" >&2; exit 1; }
# kind_of <zip path>: cpu or cuda, from the name; sets ART (the CI artifact) and JOB (the job that must have passed)
kind_of() {
    case "$(basename "$1")" in
      *-cpu.zip)  KIND=cpu;  ART=hazync-host-windows-x86_64-cpu;  JOB=prove ;;
      *-cuda.zip) KIND=cuda; ART=hazync-host-windows-x86_64-cuda; JOB=build-cuda ;;
      *) die "$(basename "$1"): want a name ending -cpu.zip or -cuda.zip" ;;
    esac
}
DISPATCH="gh workflow run windows-prove.yml --ref main -f release_build=true      # builds BOTH (CPU: the prove job, ~40 min; CUDA: about two hours)"

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
    if [ "$KIND" = cuda ]; then
        ls "$d"/cudart64_*.dll >/dev/null 2>&1 || die "no cudart64_*.dll beside host.exe"
        # reported, not refused: the CUDA host imports the Visual C++ runtime; the installer puts that runtime beside
        # it (windows-installer.yml), and a hand download needs the VC++ redistributable
        python3 scripts/pe_imports.py "$d/host.exe" --cuda 2>&1 | sed 's/^/       /' >&2 || true
    else
        ! ls "$d"/*.dll >/dev/null 2>&1 || die "the CPU host comes alone: no DLL belongs beside it"
        # PE import names are ASCII; a CPU host that imports the driver's DLL would refuse to start on a machine with
        # no NVIDIA driver, which is the machine it exists for.
        python3 -c "import sys;sys.exit(1 if b'nvcuda.dll' in open(sys.argv[1],'rb').read().lower() else 0)" "$d/host.exe" \
            || die "the CPU host.exe names nvcuda.dll: it is a CUDA build, not the CPU one"
        # ⛔ and it needs no DLL Windows does not ship (review of a04038ce, M1): no Visual C++ runtime — the CPU host
        # comes alone, for machines that may not have one (windows-prove builds it with +crt-static)
        python3 scripts/pe_imports.py "$d/host.exe" --cpu > "$d/.imports.txt" 2>&1 \
            || { sed 's/^/       /' "$d/.imports.txt" >&2; die "the CPU host.exe needs a DLL Windows does not have"; }
    fi
    python3 scripts/embedded_guest_sha.py "$d/host.exe" --pin "$WANT_SHA" --size "$size" > "$d/.guest.txt" 2>&1 \
        || { sed 's/^/       /' "$d/.guest.txt" >&2; die "host.exe does not embed the published mainnet guest"; }
    # The id as [u32;8] little-endian. NOT "exactly once" as on Linux (scripts/embeds-method-id.sh):
    # MSVC keeps two copies of the constant — measured, 2 in the host.exe of windows-prove run
    # 42540389071 — so require at least one. The guest's sha256 above is the binding check.
    local n
    n=$(python3 -c "import sys;print(open(sys.argv[1],'rb').read().count(bytes.fromhex(sys.argv[2])))" "$d/host.exe" "$WANT_ID") \
        || die "could not read host.exe"
    [ "${n:-0}" -ge 1 ] || die "host.exe does not carry the published image id ${WANT_ID:0:8}"
    echo "  ok   $KIND host.exe embeds the published guest (${WANT_SHA:0:16}…, $size bytes) and image id ${WANT_ID:0:8} (${n}x)"
    if [ "$KIND" = cuda ]; then
        # hazync#727: no system-scope atomics in any GPU module. The Windows driver refuses to LOAD a module
        # whose sm_61 image carries them on a card without host-native atomics (GTX 1050 Ti: error 801).
        # The CUDA host only: the CPU host has no GPU modules, so --require 61 could only fail it.
        # --require 61: the marker lives in the sm_61 image, so every module must have one to be seen into.
        python3 scripts/gpu_code_archs.py "$d/host.exe" --require 61 > "$d/.gpu.txt" 2>&1 \
            || { sed 's/^/       /' "$d/.gpu.txt" >&2; die "host.exe fails the GPU-module check (system-scope atomics, hazync#727)"; }
        echo "  ok   host.exe's GPU modules carry no system-scope atomics (hazync#727)"
    fi
}

case "${1:-}" in
  fetch)
    SHA="${2:?usage: $0 fetch <commit-sha> <out.zip> <guest-elf>}"; OUT="${3:?out.zip}"; ELF="${4:?guest-elf}"
    [[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || die "want a full 40-hex commit sha, got '$SHA'"
    kind_of "$OUT"
    # Newest first. A run counts only when ITS build-cuda job succeeded and the artifact is unexpired;
    # the run's own conclusion is not enough (a failed CPU prove job would hide a good CUDA build, and
    # the artifact is uploaded `if: always()`, so it can exist for a failed build).
    run_id=""
    for r in $(gh run list -R "$REPO_SLUG" --workflow windows-prove.yml --limit 50 --json databaseId,headSha \
                 --jq "[.[]|select(.headSha==\"$SHA\")|.databaseId][]" 2>/dev/null); do
        # Only a RELEASE BUILD (dispatched with release_build=true: no restored caches, --locked; the workflow puts it in
        # the run's name). A cached test build of the same commit is not what a release ships (review 11-12, release #5).
        title=$(gh api "repos/$REPO_SLUG/actions/runs/$r" --jq .display_title 2>/dev/null)
        case "$title" in *"RELEASE BUILD"*) ;; *) echo "  --   run $r: not a release build ('${title:-?}'), skipped"; continue ;; esac
        jc=$(gh api "repos/$REPO_SLUG/actions/runs/$r/jobs" --jq "[.jobs[]|select(.name==\"$JOB\")|.conclusion][0]" 2>/dev/null)
        [ "$jc" = success ] || { echo "  --   run $r: $JOB is '${jc:-absent}', skipped"; continue; }
        live=$(gh api "repos/$REPO_SLUG/actions/runs/$r/artifacts" \
                 --jq "[.artifacts[]|select(.name==\"$ART\" and .expired==false)]|length" 2>/dev/null)
        [ "$live" = 1 ] || { echo "  --   run $r: no unexpired $ART artifact, skipped"; continue; }
        run_id=$r; break
    done
    [ -n "$run_id" ] || die "no windows-prove run of ${SHA:0:8} with a successful $JOB job and a live $ART artifact.
       Dispatch a RELEASE BUILD on the release commit (NOT skip_cpu_prove: the CPU host comes from its prove job):
           $DISPATCH"
    echo "  windows-prove run $run_id (${SHA:0:8}): downloading $ART…"
    W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
    gh run download "$run_id" -R "$REPO_SLUG" -n "$ART" -D "$W/a" || die "could not download $ART from run $run_id"
    # Exactly the two kinds of file the workflow uploads, flat. Anything else is not this artifact.
    if [ "$KIND" = cuda ]; then
        extra=$(cd "$W/a" && find . -mindepth 1 \( -type d -o -type f \) ! -name host.exe ! -name 'cudart64_*.dll' -print)
    else
        extra=$(cd "$W/a" && find . -mindepth 1 \( -type d -o -type f \) ! -name host.exe -print)
    fi
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
    kind_of "$Z"
    W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
    python3 - "$Z" "$W" "$KIND" <<'PY' || die "$Z is not the expected zip ($KIND: host.exe$([ "$KIND" = cuda ] && echo ' + cudart64_*.dll'), flat)"
import fnmatch, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
names = z.namelist()
if sys.argv[3] == "cpu":
    ok = names == ["host.exe"]
else:
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
