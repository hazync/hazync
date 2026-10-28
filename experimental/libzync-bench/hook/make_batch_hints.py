"""Turn a COLLECT-mode chunk-profile log into batch-ECDSA hint files.

Usage: python3 -I make_batch_hints.py <collect.log> <out dir>

For each chunk ("batch: COLLECT for inputs LO..HI" in the host log), every following
"BATCHREC<kind> <hex u1 u2 qx qy r>" (kind 0 ECDSA, 1 Schnorr, 3 / 4 BIP341 tweak check with an even /
odd output key) line becomes one 68-byte hint, in call order:
    u32 LE valid flag | R.x (32 B BE) | R.y (32 B BE),   R = u1*G + u2*Q
valid = R is not infinity and R.x mod n == r, i.e. exactly libsecp's verdict. Invalid calls (e.g.
CHECKMULTISIG trying a signature against the wrong key) get flag 0 and are verified by libsecp.

Pure Python, Jacobian coordinates. The guest checks everything it is given, so this file is
untrusted input to it: a bug here can only make a chunk fail, never make a bad signature pass.
"""
import os
import re
import sys

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
G = (0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798,
     0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8)


def jdouble(p):
    x, y, z = p
    if y == 0 or z == 0:
        return (0, 1, 0)
    s = 4 * x * y * y % P
    m = 3 * x * x % P
    nx = (m * m - 2 * s) % P
    return (nx, (m * (s - nx) - 8 * pow(y, 4, P)) % P, 2 * y * z % P)


def jadd(p, q):
    if p[2] == 0:
        return q
    if q[2] == 0:
        return p
    z1s, z2s = p[2] * p[2] % P, q[2] * q[2] % P
    u1, u2 = p[0] * z2s % P, q[0] * z1s % P
    s1, s2 = p[1] * z2s * q[2] % P, q[1] * z1s * p[2] % P
    if u1 == u2:
        return jdouble(p) if s1 == s2 else (0, 1, 0)
    h, r = (u2 - u1) % P, (s2 - s1) % P
    h2 = h * h % P
    h3 = h * h2 % P
    nx = (r * r - h3 - 2 * u1 * h2) % P
    return (nx, (r * (u1 * h2 - nx) - s1 * h3) % P, h * p[2] * q[2] % P)


def mul(k, pt):
    acc, base = (0, 1, 0), (pt[0], pt[1], 1)
    while k:
        if k & 1:
            acc = jadd(acc, base)
        base = jdouble(base)
        k >>= 1
    return acc


def affine(p):
    if p[2] == 0:
        return None
    zi = pow(p[2], P - 2, P)
    return (p[0] * zi * zi % P, p[1] * zi * zi * zi % P)


def main(log, out):
    os.makedirs(out, exist_ok=True)
    chunks, cur = [], None
    for line in open(log, errors="replace"):
        m = re.search(r"batch: COLLECT for inputs (\d+)\.\.(\d+)", line)
        if m:
            cur = (m.group(1), m.group(2), [])
            chunks.append(cur)
            continue
        m = re.search(r"BATCHREC([0134]) ([0-9a-f]{320})", line)
        if m:
            assert cur is not None, "BATCHREC before any COLLECT header"
            cur[2].append((int(m.group(1)), bytes.fromhex(m.group(2))))
    assert chunks, "no COLLECT chunks found in the log"
    for lo, hi, recs in chunks:
        blob, valid = bytearray(), 0
        for kind, rec in recs:
            u1, u2, qx, qy, r = (int.from_bytes(rec[32 * k:32 * k + 32], "big") for k in range(5))
            R = affine(jadd(mul(u1, G), mul(u2, (qx, qy))))
            if kind == 0:   # ECDSA: x(R) mod n == r
                ok = R is not None and R[0] % N == r
            elif kind == 1: # BIP340: x(R) == r_x and y(R) even
                ok = R is not None and R[0] == r and R[1] % 2 == 0
            else:           # BIP341 tweak: R == output key (x == r, parity == kind - 3)
                ok = R is not None and R[0] == r and R[1] % 2 == kind - 3
            valid += ok
            blob += (1 if ok else 0).to_bytes(4, "little")
            blob += (R[0] if R else 0).to_bytes(32, "big") + (R[1] if R else 0).to_bytes(32, "big")
        path = f"{out}/batch_{lo}_{hi}.bin"
        open(path, "wb").write(blob)
        print(f"{path}: {len(recs)} verifies, {valid} valid, {len(recs) - valid} left to libsecp")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
