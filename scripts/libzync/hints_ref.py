#!/usr/bin/env python3
"""An independent SwiftSync hints file generator, written from 2140-dev/swiftsync-bips hintsfile.md (not from the
zyncAPP's code), for the hints zyncAPP's tests (docs/libzync/HINTS_ZAPP.md §8).

It replays the outpoint set of blocks 1..H from the block files (prover/fetch_block_rpc.py's JSON), gives every output
of every block an index except unspendable ones (script over 10,000 bytes, OP_RETURN first, the two BIP30-overwritten
coinbases), lists the indices still unspent at H and writes the file: magic, version 0x00, H, one Elias-Fano list per
block.

  hints_ref.py vectors <elias_fano.json>                      check the encoder against the BIP's test vectors
  hints_ref.py make <block dir> <H> <out file> [--order bip|crate] [--indices <out.json>]
  hints_ref.py from-indices <indices.json> <H> <out> [--order bip|crate]

--order bip: each byte filled from its most significant bit (the BIP's test vectors); crate: from bit 0 (the
hintsfile crate 0.2.0). Prints HINTS_REF <sha256> <bytes> counted=<n> unspent=<n> value=<sat>.
"""
import hashlib, json, sys

BIP30 = {91722: "00000000000271a2dc26e7667f8419f2e15416dc6955e5a6c6cdf3f2574dd08e",
         91812: "00000000000af0aed4792b1acee3d966af36cf5def14935db8de83d6f9306f2f"}


def dsha(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def varint(b, o):
    f = b[o]
    if f < 0xfd:
        return f, o + 1
    w = {0xfd: 2, 0xfe: 4, 0xff: 8}[f]
    return int.from_bytes(b[o + 1:o + 1 + w], "little"), o + 1 + w


def ser_varint(n):
    if n < 0xfd:
        return bytes([n])
    if n <= 0xffff:
        return b"\xfd" + n.to_bytes(2, "little")
    return b"\xfe" + n.to_bytes(4, "little")


def parse_tx(raw):
    """-> (txid hex display, [(prev txid display, vout)], [(value, script bytes)])"""
    o = 4
    seg = raw[4] == 0 and raw[5] == 1
    if seg:
        o += 2
    nin, o = varint(raw, o)
    ins = []
    for _ in range(nin):
        pt = raw[o:o + 32][::-1].hex()
        pv = int.from_bytes(raw[o + 32:o + 36], "little")
        o += 36
        l, o = varint(raw, o)
        o += l + 4
        ins.append((pt, pv))
    nout, o = varint(raw, o)
    outs = []
    for _ in range(nout):
        v = int.from_bytes(raw[o:o + 8], "little")
        l, o2 = varint(raw, o + 8)
        outs.append((v, raw[o2:o2 + l]))
        o = o2 + l
    body_end = o
    if seg:
        for _ in range(nin):
            k, o = varint(raw, o)
            for _ in range(k):
                l, o = varint(raw, o)
                o += l
    assert len(raw) == o + 4, "trailing bytes"
    stripped = raw[:4] + (raw[6:body_end] if seg else raw[4:body_end]) + raw[o:o + 4]
    return dsha(stripped)[::-1].hex(), ins, outs


def block_hash(b):
    h = (b["version"] & 0xffffffff).to_bytes(4, "little") + bytes.fromhex(b["prev"])[::-1] + bytes.fromhex(b["merkle"])[::-1] \
        + b["time"].to_bytes(4, "little") + b["bits"].to_bytes(4, "little") + b["nonce"].to_bytes(4, "little")
    return dsha(h)[::-1].hex()


def unspendable(spk):
    return len(spk) > 10000 or (len(spk) > 0 and spk[0] == 0x6a)


class Bits:
    def __init__(self, msb):
        self.out, self.cur, self.k, self.msb = bytearray(), 0, 0, msb

    def push(self, bit):
        if bit:
            self.cur |= (0x80 >> self.k) if self.msb else (1 << self.k)
        self.k += 1
        if self.k == 8:
            self.out.append(self.cur)
            self.cur, self.k = 0, 0

    def done(self):
        if self.k:
            self.out.append(self.cur)
        return bytes(self.out)


def ef(idx, msb):
    """hintsfile.md: CompactSize n, CompactSize m, L (n * l low bits), H (unary gaps of the high bits)."""
    if not idx:
        return b"\x00"
    n, m = len(idx), idx[-1]
    l = ((m + 1) // n).bit_length() - 1
    lo, hi = Bits(msb), Bits(msb)
    for x in idx:
        for s in range(l):
            lo.push((x >> s) & 1)
    prev = 0
    for x in idx:
        for _ in range(prev, x >> l):
            hi.push(0)
        hi.push(1)
        prev = x >> l
    return ser_varint(n) + ser_varint(m) + lo.done() + hi.done()


def write_file(lists, stop, msb):
    return bytes([0x55, 0x54, 0x58, 0x4f, 0x00]) + stop.to_bytes(4, "little") + b"".join(ef(l, msb) for l in lists)


def make(d, stop, signet=False):
    utxo = {}       # (txid, vout) -> (height, index, value)
    for h in range(1, stop + 1):
        b = json.load(open(f"{d}/block_{h}.json"))
        assert b["height"] == h
        raws = [bytes.fromhex(b["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in b["txs"]]
        over = not signet and BIP30.get(h) == block_hash(b)
        idx = 0
        for ti, raw in enumerate(raws):
            txid, ins, outs = parse_tx(raw)
            if ti > 0:
                for k in ins:
                    utxo.pop(k)   # KeyError = not live: a wrong block file
            if ti == 0 and over:
                continue
            for v, (val, spk) in enumerate(outs):
                if unspendable(spk):
                    continue
                utxo[(txid, v)] = (h, idx, val)
                idx += 1
        yield h, idx
    yield None, utxo


def main():
    a = sys.argv[1:]
    order = a[a.index("--order") + 1] if "--order" in a else "bip"
    msb = order == "bip"
    if a[0] == "vectors":
        bad = 0
        for idx, want in json.load(open(a[1])):
            got = ef(idx, True).hex()
            bad += got != want
            print(("OK  " if got == want else "BAD ") + want + ("" if got == want else " got " + got))
        print("HINTS_REF_VECTORS " + ("OK" if bad == 0 else f"FAIL {bad}"))
        sys.exit(bad != 0)
    if a[0] == "make":
        d, stop, out = a[1], int(a[2]), a[3]
        counted = 0
        utxo = None
        for h, x in make(d, stop, "--signet" in a):
            if h is None:
                utxo = x
            else:
                counted += x
        lists = [[] for _ in range(stop)]
        for (h, i, _v) in utxo.values():
            lists[h - 1].append(i)
        for l in lists:
            l.sort()
        if "--indices" in a:
            json.dump(lists, open(a[a.index("--indices") + 1], "w"))
        value = sum(v for (_h, _i, v) in utxo.values())
    elif a[0] == "from-indices":
        lists, stop, out = json.load(open(a[1])), int(a[2]), a[3]
        counted, value = -1, -1
    else:
        sys.exit(__doc__)
    f = write_file(lists, stop, msb)
    open(out, "wb").write(f)
    print(f"HINTS_REF {hashlib.sha256(f).hexdigest()} {len(f)} counted={counted} unspent={sum(len(l) for l in lists)} value={value} order={order}")


if __name__ == "__main__":
    main()
