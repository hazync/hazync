"""Turn a SwiftSync COLLECT-mode aggregate log into one ECMH hint file.

Usage: python3 -I make_ecmh_hints.py <collect.log> <out file>

Every "ECMHREC <hex leaf>" line, in order, becomes:
    u32 LE ctr | y (32 B BE) | ctr x w (32 B BE each)
where x = SHA-256(leaf || ctr_le32) is the first counter on the curve, y^2 = x^3 + 7, and each w
satisfies w^2 = -(x_k^3 + 7) for a skipped counter k (proof that x_k was not on the curve).
The guest re-checks all of it, so this file is untrusted input.
"""
import hashlib
import re
import sys

P = 2**256 - 2**32 - 977


def sqrt(a):
    r = pow(a, (P + 1) // 4, P)
    return r if r * r % P == a % P else None


def hint(leaf):
    skipped = []
    ctr = 0
    while True:
        x = int.from_bytes(hashlib.sha256(leaf + ctr.to_bytes(4, "little")).digest(), "big")
        assert x < P, "hash >= p"
        rhs = (x * x * x + 7) % P
        y = sqrt(rhs)
        if y is not None:
            out = ctr.to_bytes(4, "little") + y.to_bytes(32, "big")
            return out + b"".join(w.to_bytes(32, "big") for w in skipped)
        w = sqrt((-rhs) % P)
        assert w is not None, "p = 3 mod 4: -rhs must be a square"
        skipped.append(w)
        ctr += 1


leaves = [bytes.fromhex(m.group(1)) for line in open(sys.argv[1], errors="replace")
          for m in [re.search(r"ECMHREC ([0-9a-f]{64})", line)] if m]
assert leaves, "no ECMHREC lines in the log"
blob = b"".join(hint(l) for l in leaves)
open(sys.argv[2], "wb").write(blob)
print(f"{sys.argv[2]}: {len(leaves)} ECMH operations, {len(blob)} bytes")
