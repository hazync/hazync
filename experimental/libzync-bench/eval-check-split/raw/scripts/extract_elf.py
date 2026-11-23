# extract_elf.py <release CPU host> <out> <sha256>: the canonical guest (R0BF ProgramBinary) is embedded verbatim
# in the host (prover/methods/build.rs embed_prebuilt_guest); find it by magic + header lengths, keep the copy
# whose sha256 is the pinned GUEST_ELF_SHA256.
import hashlib, os, struct, sys
b = open(sys.argv[1], "rb").read(); want = sys.argv[3]; i = 0
while True:
    i = b.find(b"R0BF", i)
    if i < 0: sys.exit("no R0BF blob with the pinned sha256")
    try:
        ver, hl = struct.unpack_from("<II", b, i + 4); ul = struct.unpack_from("<I", b, i + 12 + hl)[0]
        k = i + 16 + hl + ul
        if b[k:k + 4] == b"\x7fELF":
            shoff = struct.unpack_from("<I", b, k + 0x20)[0]; shes, shn = struct.unpack_from("<HH", b, k + 0x2e)
            blob = b[i:k + shoff + shes * shn]
            if hashlib.sha256(blob).hexdigest() == want:
                os.makedirs(os.path.dirname(sys.argv[2]), exist_ok=True); open(sys.argv[2], "wb").write(blob)
                print(f"guest at offset {i}, {len(blob)} bytes, sha256 {want}"); sys.exit(0)
    except struct.error:
        pass
    i += 4
