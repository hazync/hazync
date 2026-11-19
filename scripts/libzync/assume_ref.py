#!/usr/bin/env python3
"""assume_ref.py [--network mainnet|signet] <snapshot> <headers.bin> [tracked.txt] -- an INDEPENDENT implementation of the assumeutxo check's
hashing (docs/libzync/ASSUME_ZAPPS.md §3), written from that document and Core's snapshot format, not from
zapps/assume/src/lib.rs.

A REFERENCE TOOL for cross-checking the zAPP's numbers, NOT a checker: it verifies no proof and compares nothing with
one. It refuses malformed input (with an error, exit 1) as Core's reader would, but use `zapp-assume assumeutxo` to
check a snapshot.

Reads a Core `dumptxoutset` file and the raw 80-byte headers from genesis to its base block, and prints:
  base <block hash>  height <H>  coins <n>  value <sat>
  hash_serialized_3 <hex>      (Core's own UTXO-set hash, as dumptxoutset prints txoutset_hash)
  live_hash x <hex> y <hex>    (the live-coin ECMH, big-endian coordinates; "identity" for the empty set)
The headers must link and the last one must be the snapshot's base block. Python standard library only; slow
(pure-Python EC arithmetic), meant for low heights.
"""
import hashlib, sys

P = 2**256 - 2**32 - 977
U32, U64 = 2**32 - 1, 2**64 - 1
MAX_SIZE = 0x02000000           # Core's CompactSize bound
MAX_MONEY = 21_000_000 * 100_000_000
MAX_SCRIPT_SIZE = 10_000
MAGIC = {"mainnet": bytes.fromhex("f9beb4d9"), "signet": bytes.fromhex("0a03cf40")}


class Bad(Exception):
    """Malformed input: always raised (an `assert` vanishes under python -O)."""


def need(cond, what):
    if not cond:
        raise Bad(what)


def sha256(b):
    return hashlib.sha256(b).digest()


def dsha(b):
    return sha256(sha256(b))


class Src:
    def __init__(self, b):
        self.b, self.i = b, 0

    def take(self, n):
        need(self.i + n <= len(self.b), f"truncated at byte {self.i}")
        r = self.b[self.i:self.i + n]
        self.i += n
        return r

    def compact(self):
        c = self.take(1)[0]
        if c < 253:
            return c
        n = {253: 2, 254: 4, 255: 8}[c]
        v = int.from_bytes(self.take(n), "little")
        need(v >= {253: 253, 254: 0x10000, 255: 0x100000000}[c], "non-canonical CompactSize")
        need(v <= MAX_SIZE, f"CompactSize {v} over MAX_SIZE")
        return v

    def varint(self, maxv):  # Core's ReadVarInt into an integer of range maxv: MSB base-128, each continuation adds one
        n = 0
        while True:
            ch = self.take(1)[0]
            need(n <= maxv >> 7, "VARINT too large")
            n = (n << 7) | (ch & 0x7F)
            if ch & 0x80:
                need(n != maxv, "VARINT too large")
                n += 1
            else:
                return n


def decompress_amount(x):
    if x == 0:
        return 0
    x -= 1
    e, x = x % 10, x // 10
    if e < 9:
        d, x = x % 9 + 1, x // 9
        n = x * 10 + d
    else:
        n = x + 1
    return n * 10**e


def script(s):
    n = s.varint(U32)
    if n == 0:
        return b"\x76\xa9\x14" + s.take(20) + b"\x88\xac"
    if n == 1:
        return b"\xa9\x14" + s.take(20) + b"\x87"
    if n in (2, 3):
        return bytes([33, n]) + s.take(32) + b"\xac"
    if n in (4, 5):
        xb = s.take(32)
        x = int.from_bytes(xb, "big")
        y = pow((x**3 + 7) % P, (P + 1) // 4, P)
        need(x < P and y * y % P == (x**3 + 7) % P, "compressed key not on the curve")
        if y % 2 != n - 4:
            y = P - y
        return b"\x41\x04" + xb + y.to_bytes(32, "big") + b"\xac"
    need(n - 6 <= MAX_SCRIPT_SIZE, f"script of {n - 6} bytes: over MAX_SCRIPT_SIZE (unspendable, never in a UTXO set)")
    return s.take(n - 6)


def ecmh_point(leaf):
    ctr = 0
    while True:
        x = int.from_bytes(sha256(leaf + ctr.to_bytes(4, "little")), "big")
        need(x < P, "ECMH: hash >= p")
        rhs = (x**3 + 7) % P
        y = pow(rhs, (P + 1) // 4, P)
        if y * y % P == rhs:
            return x, (y if y % 2 == 0 else P - y)
        ctr += 1


def add(a, b):
    if a is None:
        return b
    if a[0] == b[0]:
        if (a[1] + b[1]) % P == 0:
            return None
        lam = 3 * a[0] * a[0] * pow(2 * a[1], P - 2, P) % P
    else:
        lam = (b[1] - a[1]) * pow(b[0] - a[0], P - 2, P) % P
    x = (lam * lam - a[0] - b[0]) % P
    return x, (lam * (a[0] - x) - a[1]) % P


def compact_bytes(n):
    if n < 253:
        return bytes([n])
    return b"\xfd" + n.to_bytes(2, "little") if n <= 0xFFFF else b"\xfe" + n.to_bytes(4, "little")


def varint_bytes(n):  # Core's WriteVarInt: the coins-database key's vout encoding
    out = [n & 0x7F]
    while n > 0x7F:
        n = (n >> 7) - 1
        out.append((n & 0x7F) | 0x80)
    return bytes(reversed(out))


def main():
    args = sys.argv[1:]
    network = "mainnet"
    if "--network" in args:
        i = args.index("--network")
        network = args[i + 1]
        del args[i:i + 2]
    need(network in MAGIC, f"unknown network {network!r}")
    snap = open(args[0], "rb").read()
    hdrs = open(args[1], "rb").read()
    tracked = set()
    if len(args) > 2:
        for line in open(args[2]):
            line = line.split("#")[0].strip()
            if line:
                t, v = line.split(":")
                tracked.add((bytes.fromhex(t)[::-1], int(v)))
    need(len(hdrs) % 80 == 0 and hdrs, "headers: not a positive multiple of 80 bytes")
    hashes, times = [], []
    for k in range(len(hdrs) // 80):
        h = hdrs[80 * k:80 * k + 80]
        need(k == 0 or h[4:36] == hashes[-1], f"header {k} does not link")
        hashes.append(dsha(h))
        times.append(int.from_bytes(h[68:72], "little"))
    H = len(hashes) - 1

    def mtp(h):  # median of the timestamps of blocks max(0, h-11) .. h-1
        t = sorted(times[max(0, h - 11):h])
        return t[len(t) // 2]

    s = Src(snap)
    need(s.take(5) == b"utxo\xff", "not a Core UTXO snapshot (magic)")
    need(int.from_bytes(s.take(2), "little") == 2, "snapshot version is not 2")
    need(s.take(4) == MAGIC[network], f"the snapshot's network magic is not {network}'s")
    base = s.take(32)
    need(base == hashes[-1], "the headers do not end at the snapshot's base block")
    left = count = int.from_bytes(s.take(8), "little")
    acc, value = None, 0
    h3 = hashlib.sha256()
    last_txid = None
    while left:
        txid = s.take(32)
        need(last_txid is None or txid > last_txid, "txids not ascending: not Core's key order")
        last_txid = txid
        k = s.compact()
        need(0 < k <= left, f"{k} coins in a group with {left} left")
        last_key = None
        for _ in range(k):
            vout = s.compact()
            need(vout < U32, f"vout {vout} out of range")
            key = varint_bytes(vout)
            need(last_key is None or key > last_key, "encoded vouts not ascending: not Core's key order")
            last_key = key
            code = s.varint(U32)
            height, cb = code >> 1, code & 1
            amount = decompress_amount(s.varint(U64))
            need(amount <= MAX_MONEY, f"amount {amount} outside MoneyRange")
            spk = script(s)
            need(1 <= height <= H, f"coin at height {height}, outside 1..{H}")
            flags = cb | (2 if (txid, vout) in tracked else 0)
            leaf = sha256(b"\x00" + txid + vout.to_bytes(4, "little") + amount.to_bytes(8, "little")
                          + len(spk).to_bytes(4, "little") + spk + height.to_bytes(4, "little")
                          + bytes([flags]) + mtp(height).to_bytes(4, "little"))
            acc = add(acc, ecmh_point(leaf))
            value += amount
            h3.update(txid + vout.to_bytes(4, "little") + code.to_bytes(4, "little") + amount.to_bytes(8, "little")
                      + compact_bytes(len(spk)) + spk)
        left -= k
    need(s.i == len(snap), "bytes after the last coin")
    print(f"base {base[::-1].hex()}  height {H}  coins {count}  value {value}")
    print(f"hash_serialized_3 {sha256(h3.digest())[::-1].hex()}")
    print("live_hash identity" if acc is None else f"live_hash x {acc[0]:064x} y {acc[1]:064x}")


if __name__ == "__main__":
    try:
        main()
    except Bad as e:
        print(f"assume_ref.py: refused: {e}", file=sys.stderr)
        sys.exit(1)
