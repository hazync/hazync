#!/usr/bin/env bash
# Every Linux binary a release ships must run on glibc 2.34 (Ubuntu 22.04, Debian 12 and later): refuse any ELF
# that needs a newer GLIBC_x.y symbol version.
#
# WHY. The hosts were always built in ubuntu:22.04 (prover/build-release.sh) and need GLIBC_2.34 at most. The
# x86_64 verifier was built with the release box's own cargo (Ubuntu 24.04) and the aarch64 one on an
# ubuntu-latest runner, so EVERY release from v0.23.0 to v0.27.0 shipped hazync-verify-x86_64-linux-gnu and
# hazync-verify-aarch64 needing GLIBC_2.39 (measured 20.11.2026 from the published assets). For the x86_64 one in
# v0.24.0-v0.26.0 the need was WEAK (it ran on 22.04, warning on every run); in v0.23.0, v0.26.1 and v0.27.0, and
# for aarch64 always, it is hard: on 22.04, the platform the hosts promise, the verifier does not start
# ("version `GLIBC_2.39' not found"). Nothing looked.
#
#   scripts/check-glibc-floor.sh <file>...          # exit 0 iff every ELF needs <= HAZYNC_GLIBC_FLOOR (2.34)
#
# Reads the ELF's version-needs table (.gnu.version_r) itself, so it needs neither binutils nor a machine of the
# binary's architecture (it checks the aarch64 verifier on x86_64). A file that is not an ELF is reported and
# skipped (scripts and wasm have no glibc floor); a missing file, or an ELF it cannot parse, fails.
set -uo pipefail
FLOOR="${HAZYNC_GLIBC_FLOOR:-2.34}"
[ "$#" -gt 0 ] || { echo "usage: $0 <file>..." >&2; exit 2; }
python3 - "$FLOOR" "$@" <<'PY'
import struct, sys

def ver(s):
    return tuple(int(x) for x in s.split("."))

def glibc_needs(path):
    """The GLIBC_x.y versions an ELF's .gnu.version_r asks the loader for; None for a file that is not an ELF."""
    d = open(path, "rb").read()
    if d[:4] != b"\x7fELF":
        return None
    if d[4] != 2:
        raise ValueError("not a 64-bit ELF")
    e = "<" if d[5] == 1 else ">"
    shoff, = struct.unpack_from(e + "Q", d, 0x28)
    shentsize, shnum = struct.unpack_from(e + "HH", d, 0x3A)
    secs = [struct.unpack_from(e + "IIQQQQIIQQ", d, shoff + i * shentsize) for i in range(shnum)]
    out = {}                                        # version -> True if some need of it is NOT weak
    if not any(sec[1] == 6 for sec in secs):        # no SHT_DYNAMIC: a static ELF loads no libc at all
        return "static"
    for (_name, typ, _fl, _addr, off, _size, link, info, _al, _es) in secs:
        if typ != 0x6FFFFFFE:                       # SHT_GNU_verneed
            continue
        stroff = secs[link][4]
        def cstr(o):
            return d[stroff + o: d.index(b"\0", stroff + o)].decode()
        p = off
        for _ in range(info):                       # Elf64_Verneed: vn_version, vn_cnt, vn_file, vn_aux, vn_next
            _v, cnt, _file, aux, nxt = struct.unpack_from(e + "HHIII", d, p)
            a = p + aux
            for _ in range(cnt):                    # Elf64_Vernaux: vna_hash, vna_flags, vna_other, vna_name, vna_next
                _h, flags, _o, name, anxt = struct.unpack_from(e + "IHHII", d, a)
                n = cstr(name)
                if n.startswith("GLIBC_") and n[6:].replace(".", "").isdigit():
                    out[n[6:]] = out.get(n[6:], False) or not (flags & 0x2)   # VER_FLG_WEAK = 0x2; hard if any entry is not weak
                a += anxt
                if not anxt:
                    break
            p += nxt
            if not nxt:
                break
    return out

floor, files = sys.argv[1], sys.argv[2:]
bad = 0
for f in files:
    try:
        need = glibc_needs(f)
    except (OSError, ValueError, IndexError, struct.error) as ex:
        print(f"  FAIL {f}: cannot read its glibc needs ({ex})"); bad += 1; continue
    if need is None:
        print(f"  --   {f}: not an ELF (no glibc floor)"); continue
    if need == "static":
        print(f"  ok   {f}: a static ELF (loads no glibc)"); continue
    if not need:
        # A dynamic ELF that names no GLIBC version at all: stripped of its version table, or linked against another
        # libc. Its floor cannot be read from it, so it is not passed.
        print(f"  FAIL {f}: a dynamic ELF with no GLIBC version needs (.gnu.version_r missing or empty): floor unknown")
        bad += 1; continue
    top = max(need, key=ver)
    if top and ver(top) > ver(floor):
        # A WEAK need still loads on an older glibc, with a "weak version not found" warning printed on every
        # run (v0.24.0-v0.26.0's verifier); a hard one does not load at all (v0.26.1, v0.27.0). Both are refused.
        kind = "" if need[top] else " (weak: it starts, but warns on every run)"
        print(f"  FAIL {f} needs GLIBC_{top}{kind}; a release binary must run on glibc {floor} (build it in ubuntu:22.04)")
        bad += 1
    else:
        print(f"  ok   {f}: needs GLIBC_{top} <= {floor}")
sys.exit(1 if bad else 0)
PY
