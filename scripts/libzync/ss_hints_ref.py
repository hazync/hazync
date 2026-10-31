#!/usr/bin/env python3
"""ss_hints_ref.py <file.hints> [expected ss_hints hex] -- an INDEPENDENT implementation of the hints-file side of
a libzync proof, written from docs/libzync/SWIFTSYNC_PROOFS.md §3, not from the Rust code.

It decodes the file (magic, version, stop height, one Elias-Fano list per height), requires the encoding to be
canonical (§3: re-encoding the decoded lists gives the same bytes, nothing after height H), and computes the value
a proof at the file's stop height must commit as `ss_hints` (§3). With an expected value it exits 1 on a mismatch.
Exit 1 also on a malformed or non-canonical file. Python standard library only.
"""
import hashlib, struct, sys

MAGIC, VERSION = b"UTXO", 0
DOMAIN = b"libzync/swiftsync-hints/v2"


class Bad(Exception):
    pass


class Reader:
    def __init__(self, b):
        self.b, self.i = b, 0

    def take(self, n):
        if self.i + n > len(self.b):
            raise Bad(f"truncated at byte {self.i}")
        out = self.b[self.i:self.i + n]
        self.i += n
        return out

    def compact(self):
        """CompactSize, shortest form required (§3)."""
        x = self.take(1)[0]
        if x < 0xFD:
            return x
        if x == 0xFD:
            v = struct.unpack("<H", self.take(2))[0]
            if v < 0xFD:
                raise Bad("non-minimal CompactSize")
            return v
        if x == 0xFE:
            v = struct.unpack("<I", self.take(4))[0]
            if v <= 0xFFFF:
                raise Bad("non-minimal CompactSize")
            return v
        raise Bad("CompactSize 0xFF does not fit a u32")


def l_of(m, n):
    return ((m + 1) // n).bit_length() - 1


def unpack_bits(data, nbits):
    return [(data[k // 8] >> (k % 8)) & 1 for k in range(nbits)]


def pack_bits(bits):
    out = bytearray((len(bits) + 7) // 8)
    for k, bit in enumerate(bits):
        out[k // 8] |= bit << (k % 8)
    return bytes(out)


def encode(idx):
    """The canonical encoding of one list (§3)."""
    out = bytearray(compact_bytes(len(idx)))
    if not idx:
        return bytes(out)
    n, m = len(idx), idx[-1]
    l = l_of(m, n)
    out += compact_bytes(m)
    low = []
    for x in idx:
        low += [(x >> s) & 1 for s in range(l)]
    high, prev = [], 0
    for x in idx:
        high += [0] * ((x >> l) - prev) + [1]
        prev = x >> l
    return bytes(out) + pack_bits(low) + pack_bits(high)


def compact_bytes(v):
    if v < 0xFD:
        return bytes([v])
    if v <= 0xFFFF:
        return b"\xfd" + struct.pack("<H", v)
    return b"\xfe" + struct.pack("<I", v)


def decode_list(r):
    n = r.compact()
    if n == 0:
        return []
    m = r.compact()
    if n > m + 1:
        raise Bad(f"n = {n} > m + 1 = {m + 1}")
    l = l_of(m, n)
    low = unpack_bits(r.take((n * l + 7) // 8), n * l)
    hbits = n + (m >> l)
    high = unpack_bits(r.take((hbits + 7) // 8), hbits)
    idx, hi, k = [], 0, 0
    for pos, bit in enumerate(high):
        if bit:
            lo = sum(low[k * l + s] << s for s in range(l))
            idx.append((hi << l) | lo)
            k += 1
        else:
            hi += 1
        if k == n:
            break
    if k != n:
        raise Bad("high bits hold fewer than n ones")
    if any(b >= c for b, c in zip(idx, idx[1:])):
        raise Bad("list is not strictly ascending")
    if idx[-1] != m:
        raise Bad(f"last element {idx[-1]} != m {m}")
    return idx


def main():
    path = sys.argv[1]
    raw = open(path, "rb").read()
    try:
        r = Reader(raw)
        if r.take(4) != MAGIC:
            raise Bad("bad magic")
        if r.take(1)[0] != VERSION:
            raise Bad("unsupported version")
        H = struct.unpack("<I", r.take(4))[0]
        h_prev, canon = bytes(32), bytearray(raw[:9])
        for k in range(1, H + 1):
            start = r.i
            idx = decode_list(r)
            enc = encode(idx)
            if enc != raw[start:r.i]:
                raise Bad(f"height {k}: list is not canonically encoded")
            canon += enc
            h_prev = hashlib.sha256(DOMAIN + h_prev + struct.pack("<II", k, len(idx))
                                    + b"".join(struct.pack("<I", x) for x in idx)).digest()
        if r.i != len(raw):
            raise Bad(f"{len(raw) - r.i} bytes after height {H}")
        assert bytes(canon) == raw
    except Bad as e:
        print(f"SS_HINTS_REF_FAIL: {path}: {e}")
        sys.exit(1)
    print(f"{path}: stop height {H}, canonical, sha256 {hashlib.sha256(raw).hexdigest()}")
    print(f"ss_hints {h_prev.hex()}")
    if len(sys.argv) > 2:
        ok = h_prev.hex() == sys.argv[2].lower()
        print("SS_HINTS_MATCH" if ok else f"SS_HINTS_MISMATCH: expected {sys.argv[2]}")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
