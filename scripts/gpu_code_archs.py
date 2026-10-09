#!/usr/bin/env python3
"""Which GPU generations does a prover binary actually hold compiled code for?

    gpu_code_archs.py host.exe                    # list them
    gpu_code_archs.py host.exe --require 61,70,80 # ...and exit 1 unless EVERY module has each
    gpu_code_archs.py --control                   # self-test, no binary needed

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
"""
import struct
import sys

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
        m = {"offset": i, "size": hs + size, "images": [], "ptx": [], "compressed": False}
        off, end = i + hs, i + hs + size
        while off + ENTRY.size <= end:
            kind, _, ehs, esz, _, _, _, _, arch, _, _, flags, _, _ = ENTRY.unpack_from(b, off)
            if ehs < ENTRY.size or ehs > 4096 or esz == 0 or off + ehs + esz > end:
                break
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


def report(path, required):
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
    if not required:
        return 0
    gone = missing(mods, required)
    if gone:
        for a, where in gone.items():
            print(f"⛔ sm_{a} is MISSING from module(s) {where} of {len(mods)}")
        print("⛔ a card of that generation has no code to run in this binary")
        return 1
    print(f"✅ every one of the {len(mods)} modules holds an image for each of: "
          f"{', '.join(f'sm_{a}' for a in required)}")
    return 0


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
    print()
    if fails:
        print(f"FAIL {fails}")
        return 1
    print("All checks passed.")
    return 0


def main(argv):
    if "--control" in argv:
        return control()
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
    return report(args[0], required)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
