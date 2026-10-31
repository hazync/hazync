#!/usr/bin/env python3
"""utxo_snapshot_check.py <snapshot.dat> --bitcoin-cli "<command>" [--state state.json]

Rebuild libzync's UTXO accumulator (docs/libzync/UTREEXO_FORMAT.md) from a Bitcoin Core `dumptxoutset` file, and
compare it with the roots a proof commits: "assumeutxo, proven" (SWIFTSYNC_PROOFS.md §6 item 2).

A snapshot has each coin's outpoint, height, coinbase flag, amount and script, but not the two things the
accumulator also needs: the coin's creation MTP and its creation order (block, transaction position, vout). Both
come from your own node, through `--bitcoin-cli` (any command line that takes RPC names as arguments): the block's
transaction list for the order, and the parent header's `mediantime` for the MTP. Nothing is taken from whoever
supplied the snapshot except the coins themselves, and a wrong coin changes the roots.

With `--state` (a ChainState JSON: a sweep's state_<H>.json, or one printed from a verified proof) it compares
num_leaves and the roots and exits 1 on any difference. Python standard library only.

Snapshot format: Core v28 `node/utxo_snapshot.h` and `rpc/blockchain.cpp` (`WriteUTXOSnapshot`): magic
`utxo\\xff`, u16 version 2, 4-byte network magic, base block hash, u64 coin count, then per txid: the txid, a
CompactSize count, and per coin a CompactSize vout and the `Coin` (VARINT height*2+coinbase, compressed amount,
compressed script, `compressor.cpp`).
"""
import hashlib, json, struct, subprocess, sys

P = 2**256 - 2**32 - 977          # secp256k1 field
TAG_LEAF, TAG_NODE = b"\x00", b"\x01"


class Reader:
    def __init__(self, f):
        self.f = f

    def take(self, n):
        b = self.f.read(n)
        if len(b) != n:
            raise ValueError("snapshot truncated")
        return b

    def compact(self):
        x = self.take(1)[0]
        if x < 0xFD:
            return x
        return int.from_bytes(self.take({0xFD: 2, 0xFE: 4, 0xFF: 8}[x]), "little")

    def varint(self):
        """Core's VARINT (serialize.h ReadVarInt): base 128, MSB first, +1 per continuation."""
        n = 0
        while True:
            b = self.take(1)[0]
            n = (n << 7) | (b & 0x7F)
            if b & 0x80:
                n += 1
            else:
                return n


def decompress_amount(x):
    if x == 0:
        return 0
    x -= 1
    e, x = x % 10, x // 10
    if e < 9:
        d, x = (x % 9) + 1, x // 9
        n = x * 10 + d
    else:
        n = x + 1
    return n * 10 ** e


def decompress_script(r):
    n = r.varint()
    if n == 0:
        return b"\x76\xa9\x14" + r.take(20) + b"\x88\xac"
    if n == 1:
        return b"\xa9\x14" + r.take(20) + b"\x87"
    if n in (2, 3):
        return b"\x21" + bytes([n]) + r.take(32) + b"\xac"
    if n in (4, 5):
        x = int.from_bytes(r.take(32), "big")
        y = pow((pow(x, 3, P) + 7) % P, (P + 1) // 4, P)
        if (y * y - x ** 3 - 7) % P:
            raise ValueError("compressed public key is not on the curve")
        if y & 1 != n - 4:
            y = P - y
        return b"\x41\x04" + x.to_bytes(32, "big") + y.to_bytes(32, "big") + b"\xac"
    n -= 6
    if n > 10_000:
        raise ValueError("over-long script in a snapshot (Core never stores one)")
    return r.take(n)


def read_snapshot(path):
    """{(txid internal bytes, vout): (height, coinbase, value, script)} and the base block hash (display hex)."""
    with open(path, "rb") as f:
        r = Reader(f)
        if r.take(5) != b"utxo\xff":
            raise ValueError("not a dumptxoutset file (magic)")
        version = struct.unpack("<H", r.take(2))[0]
        if version != 2:
            raise ValueError(f"snapshot version {version}; this reads version 2 (Core v28)")
        r.take(4)
        base = r.take(32)[::-1].hex()
        count = struct.unpack("<Q", r.take(8))[0]
        coins = {}
        while len(coins) < count:
            txid = r.take(32)
            for _ in range(r.compact()):
                vout = r.compact()
                code = r.varint()
                value = decompress_amount(r.varint())
                coins[(txid, vout)] = (code >> 1, code & 1, value, decompress_script(r))
        if f.read(1):
            raise ValueError("bytes after the last coin")
    return coins, base


def rpc(cli, *args):
    parts = cli.split()
    out = subprocess.run(parts + [str(a) for a in args], capture_output=True, text=True)
    if out.returncode:
        raise RuntimeError(f"{cli} {' '.join(map(str, args))}: {out.stderr.strip()}")
    return out.stdout.strip()


def leaf(txid, vout, value, spk, height, cb, mtp):
    return hashlib.sha256(TAG_LEAF + txid + vout.to_bytes(4, "little") + value.to_bytes(8, "little")
                          + len(spk).to_bytes(4, "little") + spk + height.to_bytes(4, "little")
                          + bytes([cb]) + mtp.to_bytes(4, "little")).digest()


def forest(leaves):
    roots = []
    for node in leaves:
        k = 0
        while k < len(roots) and roots[k] is not None:
            node = hashlib.sha256(TAG_NODE + roots[k] + node).digest()
            roots[k] = None
            k += 1
        if k == len(roots):
            roots.append(None)
        roots[k] = node
    while roots and roots[-1] is None:
        roots.pop()
    return roots


def main():
    args = sys.argv[1:]
    cli = args[args.index("--bitcoin-cli") + 1] if "--bitcoin-cli" in args else None
    state = args[args.index("--state") + 1] if "--state" in args else None
    path = [a for i, a in enumerate(args) if not a.startswith("--") and (i == 0 or not args[i - 1].startswith("--"))][0]
    if not cli:
        sys.exit("--bitcoin-cli is required: creation order and MTP come from your own node")
    coins, base = read_snapshot(path)
    H = json.loads(rpc(cli, "getblockheader", base))["height"]
    print(f"{path}: {len(coins)} coins at height {H} (block {base})")
    # Creation order and MTP from the node, for every height that still has coins.
    by_height = {}
    for (txid, vout), c in coins.items():
        by_height.setdefault(c[0], []).append((txid, vout))
    ordered = []
    for h in sorted(by_height):
        bh = rpc(cli, "getblockhash", h)
        txs = json.loads(rpc(cli, "getblock", bh, 1))["tx"]
        pos = {bytes.fromhex(t)[::-1]: i for i, t in enumerate(txs)}
        mtp = json.loads(rpc(cli, "getblockheader", json.loads(rpc(cli, "getblockheader", bh))["previousblockhash"]))["mediantime"] if h > 0 else None
        for txid, vout in by_height[h]:
            if txid not in pos:
                raise SystemExit(f"UTXO_SNAPSHOT_FAIL: coin {txid[::-1].hex()}:{vout} is not in block {h} on your node")
            hh, cb, value, spk = coins[(txid, vout)]
            ordered.append(((h, pos[txid], vout), leaf(txid, vout, value, spk, hh, cb, mtp)))
    ordered.sort()
    roots = forest([l for _, l in ordered])
    print(f"forest: {len(ordered)} leaves, roots:")
    for k, r in enumerate(roots):
        print(f"  [{k}] {r.hex() if r else '-'}")
    if state:
        st = json.load(open(state))
        g = [bytes(x) if x is not None else None for x in st["utxo_roots"]]
        while g and g[-1] is None:
            g.pop()
        ok = st["utxo_leaves"] == len(ordered) and g == roots
        print("UTXO_SNAPSHOT_MATCH: the snapshot is the UTXO set the proof commits" if ok else
              f"UTXO_SNAPSHOT_MISMATCH: proof has {st['utxo_leaves']} leaves / {len(g)} roots")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
