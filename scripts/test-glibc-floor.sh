#!/usr/bin/env bash
# scripts/check-glibc-floor.sh refuses a Linux binary that needs a glibc newer than the hosts' 2.34, and
# check-dist.sh runs it on the staged Linux assets (v0.23.0-v0.27.0 shipped a verifier needing GLIBC_2.39).
#
# Fixtures are made here, not committed: a tiny C program compiled by the local gcc (it needs GLIBC_2.34 for
# __libc_start_main on any glibc >= 2.34), then its version-needs entry renamed in place (same length, or shorter
# with a NUL) or flagged WEAK:
#   2.34 -> passes          2.39 -> refused           2.39 WEAK -> refused, and called weak
#   3.10 -> refused         2.4  -> passes (numeric, not text, comparison: "2.4" > "2.34" as text)
#   a non-ELF file -> skipped, exit 0      a missing file / a truncated ELF -> refused
# and check-dist.sh refuses a dist whose verifier needs 2.39, saying why.
#
#   scripts/test-glibc-floor.sh            # must PASS
#   scripts/test-glibc-floor.sh --control  # the version comparison cut out of a COPY of the checker: MUST FAIL
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1
CONTROL=0; [ "${1:-}" = "--control" ] && CONTROL=1
command -v gcc >/dev/null || { echo "FAIL gcc is required to make the fixtures"; exit 1; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$1" = 1 ]; then echo "  ok   $2"; else echo "  FAIL $2"; fails=$((fails+1)); fi; }
C=scripts/check-glibc-floor.sh
if [ "$CONTROL" = 1 ]; then
    sed 's/if top and ver(top) > ver(floor):/if False:/' "$C" > "$T/checker.sh"; chmod +x "$T/checker.sh"
    grep -q 'if False:' "$T/checker.sh" || { echo "FAIL could not cut the comparison out"; exit 1; }
    C="$T/checker.sh"; echo "CONTROL: the version comparison is gone -- the refusal checks below MUST fail"
fi

printf '#include <stdio.h>\nint main(void){puts("hi");return 0;}\n' > "$T/h.c"
gcc -O1 -o "$T/base" "$T/h.c" || { echo "FAIL gcc could not build the fixture"; exit 1; }
mk() {   # mk <out> <new version name | -> [weak]: copy base, rename its GLIBC_2.34 need, optionally flag it WEAK
    python3 - "$T/base" "$1" "$2" "${3:-}" <<'PY'
import struct, sys
src, out, new, weak = sys.argv[1:]
d = bytearray(open(src, "rb").read())
shoff, = struct.unpack_from("<Q", d, 0x28); shentsize, shnum = struct.unpack_from("<HH", d, 0x3A)
secs = [struct.unpack_from("<IIQQQQIIQQ", d, shoff + i * shentsize) for i in range(shnum)]
hit = 0
if new == "nover":                                  # drop DT_VERNEED from the dynamic segment (what the loader reads)
    phoff, = struct.unpack_from("<Q", d, 0x20); phentsize, phnum = struct.unpack_from("<HH", d, 0x36)
    for i in range(phnum):
        ph = struct.unpack_from("<IIQQQQQQ", d, phoff + i * phentsize)
        if ph[0] == 2:
            for k in range(ph[5] // 16):
                t, = struct.unpack_from("<q", d, ph[2] + 16 * k)
                if t == 0x6FFFFFFE:
                    struct.pack_into("<q", d, ph[2] + 16 * k, 0x60000123)     # an unknown OS tag: ignored
                    hit += 1
    open(out, "wb").write(bytes(d))
    sys.exit(0 if hit else "no DT_VERNEED to remove")
for s in secs:
    if s[1] != 0x6FFFFFFE:
        continue
    stroff, p = secs[s[6]][4], s[4]
    for _ in range(s[7]):
        _v, cnt, _f, aux, nxt = struct.unpack_from("<HHIII", d, p); a = p + aux
        for _ in range(cnt):
            _h, fl, _o, name, anxt = struct.unpack_from("<IHHII", d, a)
            o = stroff + name
            if bytes(d[o:o + 11]) == b"GLIBC_2.34\0":
                hit += 1
                if new != "-":
                    nb = ("GLIBC_" + new).encode()
                    assert len(nb) <= 10
                    d[o:o + 10] = nb + b"\0" * (10 - len(nb))
                if weak == "weak":
                    struct.pack_into("<H", d, a + 4, fl | 0x2)
            a += anxt
            if not anxt:
                break
        p += nxt
        if not nxt:
            break
if not hit:
    sys.exit("the fixture does not need GLIBC_2.34 (gcc on a glibc older than 2.34?)")
if weak == "strip":                                 # remove the section headers entirely: the binary still runs
    struct.pack_into("<Q", d, 0x28, 0); struct.pack_into("<HHH", d, 0x3A, 0, 0, 0)
elif weak:
    pass
open(out, "wb").write(bytes(d))
PY
}
mk "$T/v234" - || exit 1
mk "$T/v239" 2.39; mk "$T/v239w" 2.39 weak; mk "$T/v310" 3.10; mk "$T/v24" 2.4
mk "$T/nover" nover
mk "$T/v234s" - strip; mk "$T/v239s" 2.39 strip
printf '#!/bin/sh\necho not an elf\n' > "$T/script"
head -c 100 "$T/base" > "$T/trunc"

run() { out=$("$C" "$@" 2>&1); rc=$?; }
run "$T/v234";   check "$([ $rc = 0 ] && echo 1)" "a binary needing GLIBC_2.34 passes"
run "$T/v239";   check "$([ $rc = 1 ] && printf '%s' "$out" | grep -q 'needs GLIBC_2.39;' && echo 1)" "GLIBC_2.39 is refused (the v0.26.1/v0.27.0 verifier)"
run "$T/v239w";  check "$([ $rc = 1 ] && printf '%s' "$out" | grep -q 'GLIBC_2.39 (weak' && echo 1)" "a WEAK GLIBC_2.39 need is refused too, and called weak (the v0.24.0-v0.26.0 verifier)"
run "$T/v310";   check "$([ $rc = 1 ] && echo 1)" "GLIBC_3.10 is refused"
run "$T/v24";    check "$([ $rc = 0 ] && echo 1)" "GLIBC_2.4 passes: versions compare as numbers, not text"
run "$T/nover";  check "$([ $rc = 1 ] && printf '%s' "$out" | grep -q 'floor unknown' && echo 1)" "a dynamic ELF with no GLIBC version needs is refused (floor unknown), not passed"
run "$T/v234s";  check "$([ $rc = 0 ] && printf '%s' "$out" | grep -q 'needs GLIBC_2.34' && echo 1)" "section headers stripped: still read from the program headers (2.34, not 'static')"
run "$T/v239s";  check "$([ $rc = 1 ] && printf '%s' "$out" | grep -q 'needs GLIBC_2.39' && echo 1)" "section headers stripped, needing 2.39: refused"
if gcc -static -O1 -o "$T/static" "$T/h.c" 2>/dev/null; then
    run "$T/static"; check "$([ $rc = 0 ] && printf '%s' "$out" | grep -q 'static (no interpreter' && echo 1)" "a static ELF passes (it loads no glibc)"
else
    echo "  --   (no static libc here: the static-ELF case is not exercised)"
fi
if gcc -static-pie -O1 -o "$T/spie" "$T/h.c" 2>/dev/null; then
    run "$T/spie"; check "$([ $rc = 0 ] && printf '%s' "$out" | grep -q 'static (no interpreter' && echo 1)" "a static-pie (PT_DYNAMIC, no interpreter, no DT_NEEDED) passes as static"
else
    echo "  --   (no static-pie here: that case is not exercised)"
fi
run "$T/script"; check "$([ $rc = 0 ] && printf '%s' "$out" | grep -q 'not an ELF' && echo 1)" "a non-ELF file is skipped, and says so"
run "$T/nope";   check "$([ $rc = 1 ] && echo 1)" "a missing file is refused (not a pass by absence)"
run "$T/trunc";  check "$([ $rc = 1 ] && echo 1)" "an ELF it cannot parse is refused"
run "$T/v234" "$T/v239"; check "$([ $rc = 1 ] && echo 1)" "one bad file among good ones fails the run"

# check-dist.sh runs it on EVERY staged file, whatever its name (no name pattern to fall out of).
mkdir -p "$T/dist"; cp "$T/v239" "$T/dist/some-new-linux-asset"
out=$(HAZYNC_GLIBC_CHECKER="$C" ./scripts/check-dist.sh "$T/dist" 2>&1); rc=$?
check "$([ $rc = 1 ] && printf '%s' "$out" | grep -q 'need a newer glibc than 2.34' && echo 1)" \
      "check-dist.sh checks every staged file whatever its name: a 2.39 ELF is refused, and it says why"

# ...and every Linux verifier is BUILT on the floor: release.sh and release-sign.yml use build-linux-verifier.sh.
check "$(grep -q '^run ./scripts/build-linux-verifier.sh x86_64 ' scripts/release.sh && ! grep -qE '^run cargo build .*swiftsync-verify' scripts/release.sh && echo 1)" \
      "release.sh builds the x86_64 verifier with build-linux-verifier.sh, not this box's cargo"
check "$(grep -q './scripts/build-linux-verifier.sh aarch64 hazync-verify-aarch64' .github/workflows/release-sign.yml && echo 1)" \
      "release-sign.yml builds the aarch64 verifier with build-linux-verifier.sh"
check "$(grep -q 'check-glibc-floor.sh' scripts/build-linux-verifier.sh && grep -q 'ubuntu:22.04@sha256:0e0a0fc6' scripts/build-linux-verifier.sh && grep -q 'ubuntu:22.04@sha256:0e0a0fc6' prover/build-release.sh && echo 1)" \
      "build-linux-verifier.sh uses the hosts' pinned ubuntu:22.04 image and checks its own output"

if [ "$CONTROL" = 1 ]; then echo "CONTROL: $fails failure(s)"; else echo "$fails failure(s)"; fi
if [ "$CONTROL" = 1 ]; then [ "$fails" != 0 ]; else [ "$fails" = 0 ]; fi
