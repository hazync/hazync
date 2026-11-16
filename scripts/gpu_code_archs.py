#!/usr/bin/env python3
"""Which GPU generations does a prover binary actually hold compiled code for?

    gpu_code_archs.py host.exe                    # list them
    gpu_code_archs.py host.exe --require 61,70,80 # ...and exit 1 unless EVERY module has each
    gpu_code_archs.py host --sass                 # ...and disassemble sm_70+ images (needs nvdisasm)
    gpu_code_archs.py --control                   # self-test, no binary needed

⛔ EVERY RUN ALSO REFUSES SYSTEM-SCOPE ATOMICS (hazync#727), with or without --require. See below.

⛔⛔ WHY THIS EXISTS. The Windows prover was built with `-gencode=arch=compute_61,code=sm_61` and
nothing else, for the one card it was being debugged on. A compiled GPU image runs only on its own
generation -- an sm_61 image on 6.1 and later 6.x, never on 7.x or 8.x -- so that binary held no
native code for any RTX card, and nothing said so. It was found by reading the file (2026-10-09),
after the app had been described as working "for RTX 20/30/40/50".

⛔ A BUILD FLAG IS NOT EVIDENCE ABOUT A BINARY. `sppark`'s build script does not declare
`NVCC_APPEND_FLAGS` as an input, so changing the flag does not rebuild it and a cached build tree
keeps its old images while the log shows the new flag. This reads the artefact instead, needs no
GPU and no CUDA toolkit, and is the check the build step runs on its own output.

HOW. nvcc embeds one "fat binary" per compiled file: a 16-byte header (magic 0xBA55ED50, version 1,
header size 16, payload size) followed by one entry per target. The prover holds 52 of them, of
which only the ones in the linker's own section are FINISHED MODULES (`.nv_fatbin` in ELF; PE
truncates section names to eight characters, so `.nv_fatb`). The rest are the relocatable parts
those were linked from, in a section of their own, and are not what a card loads.

⚠ The entry table's layout is not a documented interface. It is corroborated rather than assumed:
this lists exactly what NVIDIA's own `cuobjdump` reported for the same Windows binary (five
modules, all sm_61, one with PTX), and the Linux release's list matches the flags it was built with.

⛔⛔ SYSTEM-SCOPE ATOMICS (hazync#727). `::cuda::atomic<T>` defaults to thread_scope_system. For a
Pascal image ptxas records every such instruction in the cubin's `.nv.info.<kernel>` section as
EIATTR_ATOM_SYS_INSTR_OFFSETS, and the Windows driver REFUSES TO LOAD the whole module on a GPU
without host-native atomics (cudaErrorNotSupported, 801) -- every kernel in it, the ones with no
atomics included. Measured on a GTX 1050 Ti (driver 566.14): rv32im's LookupTables did exactly
this and nothing on a build box could see it. ⇒ Every image's `.nv.info` records are read, and
every PTX entry's text, and ANY system-scope atomic fails the run.

⚠ WHAT IT CAN AND CANNOT SEE. The record is the driver's own marker, so it is the thing checked.
📏 nvcc 12.9 writes it for sm_61 only: sm_70/75/80/120 images of the same kernel carry
`ATOM.E.ADD.STRONG.SYS` in their SASS and NO record, while the sm_61 SASS shows a plain
`RED.E.ADD` for both scopes -- so a SASS grep of the Pascal image misses it, and the record
check misses a Volta+-only build. The default check therefore relies on an sm_61 image (the
Windows build's --require has 61) and
on PTX, where the scope is spelled out (`atom.add.sys.u32` vs `atom.add.gpu.u32`) and a driver
JIT for a Pascal card would produce the same refused image. Compressed entries (LZ4 block format,
flag 0x2000) are decompressed here; an entry that cannot be decoded is a FAILURE, never a skip.
⇒ A module with neither an sm_6x image nor PTX (the Linux release: sm_80+ only) is BLIND to this
and is reported as such. `--sass` closes that gap with NVIDIA's `nvdisasm` (toolkit required, and
slow: minutes for a 30 MB rv32im image), refusing `ATOM/RED ... .SYS` in one sm_70+ image per
module. 📏 On the pre-fix Linux v0.18.5 CUDA host it finds `2 ATOM.E.ADD.STRONG.SYS` in rv32im only.
"""
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile

FAT_MAGIC = 0xBA55ED50
FAT_HDR = struct.Struct("<IHHQ")                    # magic, version, header_size, payload size
ENTRY = struct.Struct("<HHIQIIHHIIIQQQ")            # 64 bytes; kind, _, header_size, size, ..., arch
KIND_PTX, KIND_IMAGE = 1, 2
COMPRESSED = 0x2000


def linked_section(b):
    """(offset, size) of the section holding the finished GPU modules, or None."""
    if b[:4] == b"\x7fELF" and b[4] == 2:
        shoff, = struct.unpack_from("<Q", b, 0x28)
        entsz, num, strndx = struct.unpack_from("<HHH", b, 0x3A)
        if not shoff or entsz < 64 or strndx >= num:
            return None
        stroff, = struct.unpack_from("<Q", b, shoff + strndx * entsz + 0x18)
        for i in range(num):
            sh = shoff + i * entsz
            name_off, = struct.unpack_from("<I", b, sh)
            if b[stroff + name_off:stroff + name_off + 11] == b".nv_fatbin\0":
                return struct.unpack_from("<QQ", b, sh + 0x18)
    elif b[:2] == b"MZ":
        pe, = struct.unpack_from("<I", b, 0x3C)
        if b[pe:pe + 4] != b"PE\0\0":
            return None
        num, = struct.unpack_from("<H", b, pe + 6)
        opt, = struct.unpack_from("<H", b, pe + 20)
        sh = pe + 24 + opt
        for i in range(num):
            if b[sh + i * 40:sh + i * 40 + 8] == b".nv_fatb":
                size, off = struct.unpack_from("<II", b, sh + i * 40 + 16)
                return off, size
    return None


def modules(b):
    """[{offset, size, images: [arch...], ptx: [arch...], compressed: bool}] for each finished module."""
    sec = linked_section(b)
    if not sec:
        return None
    lo, hi = sec[0], sec[0] + sec[1]
    out, i = [], lo
    while i + FAT_HDR.size <= hi:
        magic, ver, hs, size = FAT_HDR.unpack_from(b, i)
        # ⚠ All four fields, not the magic alone: four bytes occur by chance in 250 MB.
        if magic != FAT_MAGIC or ver != 1 or hs != FAT_HDR.size or size < ENTRY.size or i + hs + size > hi:
            i += 4
            continue
        m = {"offset": i, "size": hs + size, "images": [], "ptx": [], "compressed": False, "entries": []}
        off, end = i + hs, i + hs + size
        while off + ENTRY.size <= end:
            kind, _, ehs, esz, zsz, _, _, _, arch, _, _, flags, _, rawsz = ENTRY.unpack_from(b, off)
            if ehs < ENTRY.size or ehs > 4096 or esz == 0 or off + ehs + esz > end:
                break
            m["entries"].append({"kind": kind, "arch": arch, "at": off + ehs, "size": esz,
                                 "zsize": zsz, "rawsize": rawsz, "compressed": bool(flags & COMPRESSED)})
            if kind == KIND_IMAGE:
                m["images"].append(arch)
                m["compressed"] |= bool(flags & COMPRESSED)
            elif kind == KIND_PTX:
                m["ptx"].append(arch)
            off += ehs + esz
        out.append(m)
        i = end
    return out


def missing(mods, required):
    """{arch: [module indexes lacking it]} for every required arch some module does not have."""
    return {a: [k for k, m in enumerate(mods) if a not in m["images"]]
            for a in required if any(a not in m["images"] for m in mods)}


# ── system-scope atomics (hazync#727) ────────────────────────────────────────────────────────────
SHT_CUDA_INFO = 0x70000000
EIFMT_NVAL, EIFMT_BVAL, EIFMT_HVAL, EIFMT_SVAL = 1, 2, 3, 4
# 📏 0x27, read from nvcc 12.9 output: the one .nv.info record a system-scope kernel has and the
# same kernel at device scope does not, and the one `cuobjdump -elf` names EIATTR_ATOM_SYS_INSTR_OFFSETS.
EIATTR_ATOM_SYS_INSTR_OFFSETS = 0x27
# PTX ISA: atom{.sem}{.scope}{.space}.op.type and red{...}; scope .sys. A `.sys` FENCE is not an
# atomic and is not refused -- only atom/red, which is what the driver's marker records.
PTX_SYS_ATOMIC = re.compile(rb"^\s*(?:@!?%\w+\s+)?(?:atom|red)(?:\.\w+)*?\.sys(?:\.\w+)*\s", re.M)


def lz4_block(src, rawsize):
    """Decode one LZ4 *block* (no frame) -- the format nvcc uses for a compressed fatbin entry."""
    out, i, n = bytearray(), 0, len(src)
    while i < n:
        tok = src[i]
        i += 1
        lit = tok >> 4
        if lit == 15:
            while True:
                x = src[i]
                i += 1
                lit += x
                if x != 255:
                    break
        out += src[i:i + lit]
        i += lit
        if i >= n or len(out) >= rawsize:
            break                                 # the last sequence is literals only
        dist = src[i] | src[i + 1] << 8
        i += 2
        mlen = tok & 15
        if mlen == 15:
            while True:
                x = src[i]
                i += 1
                mlen += x
                if x != 255:
                    break
        mlen += 4
        if dist == 0 or dist > len(out):
            raise ValueError(f"LZ4 back-reference {dist} beyond the {len(out)} bytes decoded")
        start = len(out) - dist
        if dist >= mlen:
            out += out[start:start + mlen]
        else:                                     # overlapping copy: the pattern repeats
            chunk = out[start:]
            out += (chunk * (mlen // dist + 1))[:mlen]
    if len(out) != rawsize:
        raise ValueError(f"LZ4 block decoded to {len(out)} bytes, the entry says {rawsize}")
    return bytes(out)


def payload(b, e):
    """The entry's bytes, decompressed. Raises on anything it cannot decode -- never guesses."""
    data = b[e["at"]:e["at"] + e["size"]]
    if not e["compressed"]:
        return data
    zsz = e["zsize"] or e["size"]
    if not 0 < zsz <= e["size"] or not e["rawsize"]:
        raise ValueError(f"compressed entry with sizes {zsz}/{e['size']}/{e['rawsize']}")
    return lz4_block(data[:zsz], e["rawsize"])


def cubin_sys_atomics(c):
    """[(kernel, number of system-scope atomic instructions)] from a cubin's .nv.info records."""
    if c[:4] != b"\x7fELF" or c[4] != 2:
        raise ValueError("an image entry that is not a 64-bit ELF cubin")
    shoff, = struct.unpack_from("<Q", c, 0x28)
    entsz, num, strndx = struct.unpack_from("<HHH", c, 0x3A)
    if not shoff or entsz < 64 or strndx >= num or shoff + num * entsz > len(c):
        raise ValueError("a cubin with an unreadable section table")
    stroff, = struct.unpack_from("<Q", c, shoff + strndx * entsz + 0x18)
    found, infos = [], 0
    for k in range(num):
        sh = shoff + k * entsz
        name_off, typ = struct.unpack_from("<II", c, sh)
        off, size = struct.unpack_from("<QQ", c, sh + 0x18)
        if typ != SHT_CUDA_INFO:
            continue
        name = c[stroff + name_off:c.index(b"\0", stroff + name_off)].decode("utf-8", "replace")
        if not name.startswith(".nv.info"):
            continue
        infos += 1
        p, end, hits = off, off + size, 0
        while p + 4 <= end:
            fmt, attr = c[p], c[p + 1]
            if fmt == EIFMT_SVAL:
                ln, = struct.unpack_from("<H", c, p + 2)
                if attr == EIATTR_ATOM_SYS_INSTR_OFFSETS:
                    hits += max(1, ln // 4)
                p += 4 + ln
            elif fmt in (EIFMT_NVAL, EIFMT_BVAL, EIFMT_HVAL):
                p += 4
            else:
                raise ValueError(f"unknown .nv.info record format {fmt} in {name}")
        if hits:
            found.append((name[len(".nv.info."):] or "(module)", hits))
    if not infos:
        raise ValueError("a cubin with no .nv.info section -- nothing was checked")
    return found


def sys_atomics(b, mods):
    """[(module, 'sm_61'|'PTX 61', what)] for every system-scope atomic, or an undecodable entry."""
    bad = []
    for k, m in enumerate(mods):
        for e in m["entries"]:
            if e["kind"] not in (KIND_IMAGE, KIND_PTX):
                continue
            where = f"sm_{e['arch']}" if e["kind"] == KIND_IMAGE else f"PTX {e['arch']}"
            try:
                data = payload(b, e)
                if e["kind"] == KIND_IMAGE:
                    for fn, n in cubin_sys_atomics(data):
                        bad.append((k, where, f"{n} system-scope atomic(s) in {fn}"))
                else:
                    n = len(PTX_SYS_ATOMIC.findall(data))
                    if n:
                        bad.append((k, where, f"{n} atom/red .sys instruction(s) in the PTX"))
            except (ValueError, IndexError, struct.error) as err:
                bad.append((k, where, f"UNREADABLE, so unchecked: {err}"))
    return bad



SASS_SYS_ATOMIC = re.compile(r"\b(?:ATOM|ATOMG|RED|REDG)\.[A-Z0-9_.]*?\bSYS\b")


def blind_modules(mods):
    """Modules where the record-and-PTX check cannot see a system-scope atomic: no sm_6x image, no PTX."""
    return [k for k, m in enumerate(mods)
            if not m["ptx"] and not any(60 <= a < 70 for a in m["images"])]


def sass_sys_atomics(b, mods, nvdisasm):
    """[(module, 'sm_NN SASS', what)] from disassembling ONE sm_70+ image per module."""
    bad = []
    with tempfile.TemporaryDirectory() as td:
        for k, m in enumerate(mods):
            imgs = [e for e in m["entries"] if e["kind"] == KIND_IMAGE and e["arch"] >= 70]
            if not imgs:
                continue
            e = imgs[0]
            where = f"sm_{e['arch']} SASS"
            try:
                path = os.path.join(td, f"m{k}.cubin")
                with open(path, "wb") as f:
                    f.write(payload(b, e))
                r = subprocess.run([nvdisasm, path], capture_output=True, text=True)
                if r.returncode != 0 or not r.stdout.strip():
                    raise ValueError(f"nvdisasm exit {r.returncode}: {r.stderr.strip()[:200]}")
                hits = SASS_SYS_ATOMIC.findall(r.stdout)
                if hits:
                    bad.append((k, where, f"{len(hits)} system-scope instruction(s): "
                                          f"{', '.join(sorted(set(hits)))}"))
            except (ValueError, OSError) as err:
                bad.append((k, where, f"UNREADABLE, so unchecked: {err}"))
    return bad


def report(path, required, sass=False):
    b = open(path, "rb").read()
    mods = modules(b)
    if mods is None:
        print(f"⛔ {path}: no finished-GPU-module section found — is this the CUDA build?")
        return 1
    # ⛔ ZERO MODULES IS NOT "NOTHING IS MISSING". A loop over nothing passes every requirement.
    if not mods:
        print(f"⛔ {path}: the GPU section holds no fat binary this could read — nothing was checked")
        return 1
    print(f"{path}: {len(mods)} finished GPU module(s)")
    for k, m in enumerate(mods):
        print(f"  module {k}  {m['size']:>10,} bytes  images: "
              f"{', '.join(f'sm_{a}' for a in m['images']) or '(none)'}"
              f"{'  +PTX ' + ','.join(str(a) for a in m['ptx']) if m['ptx'] else ''}"
              f"{'  (compressed)' if m['compressed'] else ''}")
    rc = 0
    bad = sys_atomics(b, mods)
    if sass:
        nvdisasm = shutil.which("nvdisasm")
        if not nvdisasm:
            print("⛔ --sass needs NVIDIA's nvdisasm on PATH (CUDA toolkit) — refusing to report a check it did not run")
            return 1
        bad += sass_sys_atomics(b, mods, nvdisasm)
    if bad:
        for k, where, what in bad:
            print(f"⛔ module {k} {where}: {what}")
        print("⛔ SYSTEM-SCOPE ATOMICS (hazync#727): the Windows driver refuses to load such a module on a "
              "GPU without host-native atomics (Pascal), error 801 -- use ::cuda::thread_scope_device")
        rc = 1
    blind = [] if sass else blind_modules(mods)
    if blind:
        # ⛔ NOT a ✅: "none found" over modules this cannot see would be the flattering reading.
        print(f"⚠ module(s) {blind} hold no sm_6x image and no PTX: the record check CANNOT SEE a "
              f"system-scope atomic there (sm_70+ write none). Run with --sass for those.")
    elif not bad:
        n = sum(len(m["entries"]) for m in mods)
        print(f"✅ no system-scope atomics in any of the {n} images/PTX entries"
              f"{' or the sm_70+ SASS' if sass else ''} (hazync#727)")
    if not required:
        return rc
    gone = missing(mods, required)
    if gone:
        for a, where in gone.items():
            print(f"⛔ sm_{a} is MISSING from module(s) {where} of {len(mods)}")
        print("⛔ a card of that generation has no code to run in this binary")
        return 1
    print(f"✅ every one of the {len(mods)} modules holds an image for each of: "
          f"{', '.join(f'sm_{a}' for a in required)}")
    return rc


# ── self-test ────────────────────────────────────────────────────────────────────────────────────
def _fat(images, ptx=()):
    body = b""
    for kind, archs in ((KIND_PTX, ptx), (KIND_IMAGE, images)):
        for a in archs:
            payload = bytes(80)
            body += ENTRY.pack(kind, 0, ENTRY.size, len(payload), 0, 0, 0, 0, a, 0, 0, 0, 0, 0) + payload
    return FAT_HDR.pack(FAT_MAGIC, 1, FAT_HDR.size, len(body)) + body


def _pe(section_name, blob, decoy=b""):
    """A minimal PE with one named section holding `blob`, and `decoy` in a second section."""
    hdr = bytearray(0x400)
    hdr[0:2] = b"MZ"
    struct.pack_into("<I", hdr, 0x3C, 0x80)
    hdr[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<H", hdr, 0x80 + 6, 2)          # two sections
    struct.pack_into("<H", hdr, 0x80 + 20, 0)         # no optional header
    sh = 0x80 + 24
    for i, (name, data, off) in enumerate(((section_name, blob, 0x400),
                                           (b"__nv_rel", decoy, 0x400 + len(blob)))):
        hdr[sh + i * 40:sh + i * 40 + 8] = name.ljust(8, b"\0")
        struct.pack_into("<II", hdr, sh + i * 40 + 16, len(data), off)
    return bytes(hdr) + blob + decoy


def control():
    fails = 0

    def check(ok, what):
        nonlocal fails
        print(f"  {'ok  ' if ok else 'FAIL'} {what}")
        fails += 0 if ok else 1

    good = _pe(b".nv_fatb", _fat([61, 70, 80, 120], ptx=[61]) + _fat([61, 70, 80, 120]),
               decoy=_fat([61]))
    mods = modules(good)
    check(mods is not None and len(mods) == 2, "two finished modules are found in a PE")
    check(mods and mods[0]["images"] == [61, 70, 80, 120] and mods[0]["ptx"] == [61],
          "their images and PTX are read")
    check(mods and not missing(mods, [61, 70, 80, 120]), "a build with every generation passes")
    pascal_only = _pe(b".nv_fatb", _fat([61], ptx=[61]) + _fat([61]))
    gone = missing(modules(pascal_only), [61, 70, 80, 120])
    check(set(gone) == {70, 80, 120}, "⛔ the Pascal-only build is caught: 70, 80 and 120 are missing")
    one_short = _pe(b".nv_fatb", _fat([61, 70, 80, 120]) + _fat([61, 80, 120]))
    gone = missing(modules(one_short), [61, 70, 80, 120])
    check(gone == {70: [1]}, "⛔ ONE module lacking ONE generation is caught, and named — a stale "
                             "cached library looks exactly like this")
    # ⛔ The relocatable parts must not be able to satisfy a requirement on a module's behalf.
    decoy_has_it = _pe(b".nv_fatb", _fat([61]), decoy=_fat([61, 70, 80, 120]))
    check(set(missing(modules(decoy_has_it), [70])) == {70},
          "⛔ an image that exists only in a relocatable part does not count")
    check(modules(_pe(b".text\0\0\0", _fat([61]))) is None,
          "a binary with no GPU section is reported as such, not as zero problems")
    check(modules(_pe(b".nv_fatb", bytes(256))) == [],
          "a GPU section with nothing readable in it is an empty list — which report() refuses")

    # ⛔⛔ hazync#727: SYSTEM-SCOPE ATOMICS. Real nvcc output (scripts/testdata/atom-scope/, built
    # from the atom_scope.cu there), wrapped in a PE exactly as the linker leaves it in host.exe.
    td = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata", "atom-scope")

    def scan(*names):
        fats = []
        for n in names:
            with open(os.path.join(td, n), "rb") as f:
                fats.append(f.read())
        exe = _pe(b".nv_fatb", b"".join(fats))
        return sys_atomics(exe, modules(exe))

    dev = scan("dev.fatbin")
    check(dev == [], "a device-scope atomic module (compressed sm_61 + PTX) passes")
    sys_ = scan("sys.fatbin")
    check(any(w == "sm_61" and "count" in what for _, w, what in sys_)
          and any(w == "PTX 61" for _, w, _ in sys_),
          "⛔ the same module at SYSTEM scope is refused, in its compressed image AND its PTX")
    raw = scan("sys_raw.fatbin")
    check([w for _, w, _ in raw] == ["sm_61"], "⛔ an uncompressed sm_61 image alone is refused")
    ptx = scan("sys_ptx.fatbin")
    check([w for _, w, _ in ptx] == ["PTX 61"], "⛔ PTX alone (what a driver JIT would compile) is refused")
    mixed = scan("dev.fatbin", "dev.fatbin", "sys_raw.fatbin")
    check([k for k, _, _ in mixed] == [2], "⛔ one bad module among clean ones is caught and named")
    check(PTX_SYS_ATOMIC.search(b"\tatom.relaxed.sys.global.add.u32 %r1, [%rd1], 1;\n")
          and PTX_SYS_ATOMIC.search(b"\t@%p1 red.sys.global.add.u32 [%rd1], 1;\n")
          and not PTX_SYS_ATOMIC.search(b"\tatom.add.gpu.u32 %r1,[%rd1],%r2;\n")
          and not PTX_SYS_ATOMIC.search(b"\tfence.sc.sys;\n"),
          "the PTX pattern takes every scope spelling, and not a .sys fence or a .gpu atomic")
    # ⛔ An entry this cannot read must be a FAILURE, never "no atomics found".
    with open(os.path.join(td, "sys.fatbin"), "rb") as f:
        corrupt = bytearray(f.read())
    # The PTX entry's header starts at 16; its uncompressed size is the last field (offset 56).
    # One byte more than the stream really holds: an LZ4 block has no checksum, so the length is
    # the only consistency a decoder can hold it to, and a mismatch must not pass as "clean".
    raw, = struct.unpack_from("<Q", corrupt, 16 + 56)
    struct.pack_into("<Q", corrupt, 16 + 56, raw + 1)
    exe = _pe(b".nv_fatb", bytes(corrupt))
    check(any("UNREADABLE" in what for _, _, what in sys_atomics(exe, modules(exe))),
          "⛔ a compressed entry that does not decode to its stated size is UNREADABLE, not clean")
    check(blind_modules(modules(_pe(b".nv_fatb", _fat([80, 90]) + _fat([61, 80]) + _fat([80], ptx=[80])))) == [0],
          "⚠ a module with no sm_6x image and no PTX is reported as BLIND, the others are not")
    check(SASS_SYS_ATOMIC.search("@P0 ATOM.E.ADD.STRONG.SYS PT, R3, [R2.64], R5 ;")
          and SASS_SYS_ATOMIC.search("RED.E.ADD.STRONG.SYS [R2.64], R5 ;")
          and not SASS_SYS_ATOMIC.search("@P0 ATOM.E.ADD.STRONG.GPU PT, R3, [R2.64], R5 ;")
          and not SASS_SYS_ATOMIC.search("MEMBAR.SC.SYS ;"),
          "the SASS pattern (--sass) takes ATOM/RED .SYS, and not .GPU or a .SYS barrier")
    noinfo = _pe(b".nv_fatb", _fat([61]))             # an "image" that is not a cubin at all
    check(any("UNREADABLE" in what for _, _, what in sys_atomics(noinfo, modules(noinfo))),
          "⛔ an image that is not a readable cubin is reported, not skipped")
    print()
    if fails:
        print(f"FAIL {fails}")
        return 1
    print("All checks passed.")
    return 0


def main(argv):
    # ⛔ BEFORE ANYTHING PRINTS. A Windows runner's stdout is cp1252, which cannot encode the marks
    # this prints. Run 37961309707 built a correct binary in three hours, listed its five modules,
    # and then died on the line saying they were all present — failing the build on its own
    # success message.
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    if "--control" in argv:
        return control()
    sass = "--sass" in argv
    args = [a for a in argv if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2
    required = []
    if "--require" in argv:
        raw = argv[argv.index("--require") + 1]
        args = [a for a in args if a != raw]
        # ⛔ VALIDATE, NEVER STRIP. `--require ""` must not become "nothing is required".
        if not raw or any(not p.strip().isdigit() for p in raw.replace(" ", ",").split(",") if p != ""):
            print(f"⛔ --require wants numbers like 61,70,80 — got {raw!r}")
            return 2
        required = [int(p) for p in raw.replace(" ", ",").split(",") if p != ""]
        if not required:
            print("⛔ --require was given no generations — refusing to check nothing")
            return 2
    return report(args[0], required, sass)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
