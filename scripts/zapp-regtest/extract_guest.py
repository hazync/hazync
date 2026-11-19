#!/usr/bin/env python3
"""extract_guest.py <host binary> <out.bin>: the guest program (risc0 R0BF ProgramBinary) a libzync host embeds.

The regtest image is not published (reproduce/LIBZYNC_IDS lists mainnet and signet), so the GPU batch takes its guest
from the CI-built regtest host (build-hosts.sh host_reg, built in the reproducible container). A CUDA host is then built
with HAZYNC_GUEST_ELF=<out.bin>, and prover/methods/build.rs recomputes the image id from these bytes and refuses a
mismatch, so a wrong extraction cannot pass as the image.

R0BF layout (risc0-binfmt 3.x, ProgramBinary::encode): b"R0BF", u32 version, u32 header length, header, u32 user ELF
length, user ELF, then the kernel ELF with no length (its end is read from its own ELF32 header: the section header
table, or the last segment, whichever ends later). The host holds the program several times (6 copies in host_m and host_reg, 11-16): every copy
found must be the same bytes. Validated on host_m (CI run 43835411815, main 797b777e): all 6 copies hash to the
published mainnet GUEST_ELF_SHA256 f7b87bf1...; host_reg's 6 copies give 1c3d671a... (70,197,404 bytes).
"""
import hashlib, struct, sys


def elf32_end(b, at):
    assert b[at:at + 4] == b"\x7fELF" and b[at + 4] == 1, "not an ELF32"
    phoff, shoff = struct.unpack_from("<II", b, at + 0x1C)
    phentsize, phnum, shentsize, shnum = struct.unpack_from("<HHHH", b, at + 0x2A)
    end = shoff + shnum * shentsize
    for i in range(phnum):
        _, off, _, _, filesz = struct.unpack_from("<IIIII", b, at + phoff + i * phentsize)
        end = max(end, off + filesz)
    return at + end


def main():
    b = open(sys.argv[1], "rb").read()
    found = []
    i = b.find(b"R0BF")
    while i >= 0:
        try:
            ver, hlen = struct.unpack_from("<II", b, i + 4)
            p = i + 12 + hlen
            ulen, = struct.unpack_from("<I", b, p)
            user = p + 4
            if ver == 1 and hlen < 4096 and b[user:user + 4] == b"\x7fELF" and b[user + ulen:user + ulen + 4] == b"\x7fELF":
                found.append((i, elf32_end(b, user + ulen)))
        except (AssertionError, struct.error):
            pass
        i = b.find(b"R0BF", i + 1)
    for s, e in found:
        print(f"R0BF at {s}, {e - s} bytes, sha256 {hashlib.sha256(b[s:e]).hexdigest()}")
    blobs = {b[s:e] for s, e in found}
    assert len(blobs) == 1, f"expected one guest program, found {len(blobs)} different R0BF blobs"
    if len(sys.argv) > 2:
        open(sys.argv[2], "wb").write(blobs.pop())
        print(f"wrote {sys.argv[2]}")


main()
