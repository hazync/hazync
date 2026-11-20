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
# Reads the ELF's version needs itself, from the program headers and the dynamic segment (what the loader reads;
# section headers can be stripped), so it needs neither binutils nor a machine of the binary's architecture (it checks the aarch64 verifier on x86_64). A file that is not an ELF is reported and
# skipped (scripts and wasm have no glibc floor); a missing file, or an ELF it cannot parse, fails.
set -uo pipefail
FLOOR="${HAZYNC_GLIBC_FLOOR:-2.34}"
[ "$#" -gt 0 ] || { echo "usage: $0 <file>..." >&2; exit 2; }
python3 - "$FLOOR" "$@" <<'PY'
import re, struct, sys

def ver(s):
    return tuple(int(x) for x in s.split("."))

def glibc_needs(path):
    """The GLIBC_x.y versions an ELF asks the loader for ({version: hard?}); None for a file that is not an ELF;
    "static" for one that loads no libc.

    Read from the PROGRAM headers (what the loader reads), never the section headers (`strip` can remove those
    and the binary still runs): dynamic = a PT_INTERP, or DT_NEEDED entries in PT_DYNAMIC; the needs come from
    DT_VERNEED / DT_VERNEEDNUM, located through the PT_LOAD mappings. No interpreter and no DT_NEEDED (static,
    static-pie) = static: it loads no glibc, so it has no floor."""
    d = open(path, "rb").read()
    if d[:4] != b"\x7fELF":
        return None
    if d[4] != 2:
        raise ValueError("not a 64-bit ELF")
    e = "<" if d[5] == 1 else ">"
    phoff, = struct.unpack_from(e + "Q", d, 0x20)
    phentsize, phnum = struct.unpack_from(e + "HH", d, 0x36)
    phs = [struct.unpack_from(e + "IIQQQQQQ", d, phoff + i * phentsize) for i in range(phnum)]
    loads = [(ph[3], ph[2], ph[5]) for ph in phs if ph[0] == 1]          # PT_LOAD: (vaddr, offset, filesz)

    def off(vaddr):
        for va, fo, sz in loads:
            if va <= vaddr < va + sz:
                return fo + (vaddr - va)
        raise ValueError(f"address {vaddr:#x} is in no PT_LOAD segment")
    interp = any(ph[0] == 3 for ph in phs)                               # PT_INTERP
    dyn = [ph for ph in phs if ph[0] == 2]                               # PT_DYNAMIC
    tags = []
    if dyn:
        o, sz = dyn[0][2], dyn[0][5]
        for i in range(sz // 16):
            t, v = struct.unpack_from(e + "qQ", d, o + 16 * i)
            if t == 0:
                break
            tags.append((t, v))
    needed = any(t == 1 for t, _ in tags)                                # DT_NEEDED
    if not interp and not needed:
        return "static"
    tv = dict((t, v) for t, v in tags)
    out = {}                                        # version -> True if some need of it is NOT weak
    if 0x6FFFFFFE not in tv:                        # no DT_VERNEED
        return out
    stroff = off(tv[5])                             # DT_STRTAB

    def cstr(o):
        return d[stroff + o: d.index(b"\0", stroff + o)].decode()
    if 5 not in tv:
        raise ValueError("DT_VERNEED without DT_STRTAB")
    p = off(tv[0x6FFFFFFE])
    if not 0 < tv.get(0x6FFFFFFF, 0) <= 64:
        raise ValueError(f"DT_VERNEEDNUM {tv.get(0x6FFFFFFF, 0)} is not a believable count")
    for _ in range(tv.get(0x6FFFFFFF, 0)):          # DT_VERNEEDNUM x Elf64_Verneed
        _v, cnt, _file, aux, nxt = struct.unpack_from(e + "HHIII", d, p)
        if cnt > 256:
            raise ValueError(f"a Verneed entry claims {cnt} versions")
        a = p + aux
        for _ in range(cnt):                        # Elf64_Vernaux: vna_hash, vna_flags, vna_other, vna_name, vna_next
            _h, flags, _o, name, anxt = struct.unpack_from(e + "IHHII", d, a)
            n = cstr(name)
            if n.startswith("GLIBC_"):                  # numeric or not: GLIBC_ABI_DT_RELR, GLIBC_PRIVATE are kept
                out[n[6:]] = out.get(n[6:], False) or not (flags & 0x2)   # VER_FLG_WEAK = 0x2; hard if any is not weak
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
    except Exception as ex:  # noqa: BLE001 -- every file is reported; one malformed file must not hide the rest
        print(f"  FAIL {f}: cannot read its glibc needs ({type(ex).__name__}: {ex})"); bad += 1; continue
    if need is None:
        print(f"  --   {f}: not an ELF (no glibc floor)"); continue
    if need == "static":
        print(f"  ok   {f}: static (no interpreter, no DT_NEEDED: loads no glibc)"); continue
    if not need:
        # A dynamic ELF that names no GLIBC version at all: stripped of its version table, or linked against another
        # libc. Its floor cannot be read from it, so it is not passed.
        print(f"  FAIL {f}: a dynamic ELF with no GLIBC version needs (.gnu.version_r missing or empty): floor unknown")
        bad += 1; continue
    odd = sorted(v for v in need if not re.fullmatch(r"[0-9]+(\.[0-9]+)*", v))
    if odd:
        # Not a version number, so not comparable with the floor -- and not harmless: GLIBC_ABI_DT_RELR (packed
        # relative relocations, gcc -Wl,-z,pack-relative-relocs) exists only from glibc 2.36, GLIBC_PRIVATE is
        # glibc-internal ABI that changes between builds. A release binary has no business needing either.
        print(f"  FAIL {f} needs {', '.join('GLIBC_' + v for v in odd)}: a non-numeric glibc need, refused "
              f"(GLIBC_ABI_DT_RELR needs glibc >= 2.36; build without -z pack-relative-relocs, in ubuntu:22.04)")
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
