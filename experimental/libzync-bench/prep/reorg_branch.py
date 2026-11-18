#!/usr/bin/env python3
"""A synthetic BRANCH of mainnet for the reorg tests (docs/libzync/REORG.md §12): block files F+1..F+L that fork from
mainnet's block F, in the host's JSON format (prover/fetch_block_rpc.py).

  python3 reorg_branch.py <mainnet block dir> <F> <L> <tag> <out dir>

Prep checks no proof of work, script or merkle root (the proofs do), but it needs every coin it spends to be in its
stores exactly as created: each branch block has a coinbase of its own (its scriptSig carries the tag and the height,
so its txid is new), and every block but the first spends one coinbase output of the shared chain that is still
unspent at F (as one input of a transaction with two outputs), and spends that transaction's first output again in
the same block (an in-block spend). So the branch exercises the coin log's flips and appends, the trie's marks and
inserts and the BIP30 tree's coinbase counts, as a real block does. Headers chain by their real hashes, the times
step 600 s from F's, the bits are F's. A branch longer than the shared chain's unspent coinbases (the end-to-end test's
W = 288, reorg_e2e.py) spends the branch's OWN earlier coinbases once those run out (immature: prep does not check
maturity), so any length works. The base directory may hold synthetic blocks too (a branch of a branch).
"""
import hashlib, json, pathlib, struct, sys

def dsha(b): return hashlib.sha256(hashlib.sha256(b).digest()).digest()
def varint(n): return bytes([n]) if n < 0xfd else b"\xfd" + struct.pack("<H", n)
def push(b): return varint(len(b)) + b

def tx(inputs, outputs):
    """Legacy serialization: inputs (txid display hex, vout, scriptSig), outputs (sats, script)."""
    b = struct.pack("<i", 1) + varint(len(inputs))
    for txid, vout, ss in inputs:
        b += bytes.fromhex(txid)[::-1] + struct.pack("<I", vout) + push(ss) + b"\xff\xff\xff\xff"
    b += varint(len(outputs))
    for v, spk in outputs:
        b += struct.pack("<q", v) + push(spk)
    return b + struct.pack("<I", 0)

def txid(raw): return dsha(raw)[::-1].hex()

def header_hash(j):
    h = (struct.pack("<I", j["version"] & 0xffffffff) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
         + struct.pack("<III", j["time"], j["bits"], j["nonce"]))
    return dsha(h)[::-1].hex()

def p2pkh(tag): return b"\x76\xa9\x14" + hashlib.sha256(tag.encode()).digest()[:20] + b"\x88\xac"

def main():
    src, F, L, tag, out = pathlib.Path(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], pathlib.Path(sys.argv[5])
    out.mkdir(parents=True, exist_ok=True)
    base = {h: json.load(open(src / f"block_{h}.json")) for h in range(1, F + 1)}
    times = {0: 1231006505, **{h: b["time"] for h, b in base.items()}}
    # coinbase outputs of the shared chain, and the outpoints spent in it up to F
    spent, coins = set(), []
    for h, b in base.items():
        for t in b["txs"]:
            raw = bytes.fromhex(t["raw"])
            n_in = raw[4]                                              # < 0xfd in these early blocks
            o = 5
            for _ in range(n_in):
                spent.add((raw[o:o + 32][::-1].hex(), struct.unpack_from("<I", raw, o + 32)[0]))
                o += 36; sl = raw[o]; o += 1 + sl + 4
        cb = bytes.fromhex(b["coinbase_hex"])
        # the coinbase's single output: value and script (these blocks pay 50 BTC to one script)
        o = 4 + 1 + 32 + 4; sl = cb[o]; o += 1 + sl + 4; assert cb[o] == 1, f"block {h}: one coinbase output expected"
        o += 1; v = struct.unpack_from("<q", cb, o)[0]; o += 8; sl = cb[o]
        coins.append((h, txid(cb), v, cb[o + 1:o + 1 + sl]))
    unspent = [c for c in coins if (c[1], 0) not in spent and c[0] <= F - 100]   # mature at F (not checked; tidy)
    def mtp(h):                                                       # median time past of block h (Core's: up to 11)
        ts = sorted(times[k] for k in range(max(0, h - 10), h + 1))
        return ts[len(ts) // 2]
    prev, t = header_hash(base[F]), base[F]["time"]
    bits = base[F]["bits"]
    own = []                                                          # this branch's coinbases, oldest first
    for i in range(1, L + 1):
        h = F + i
        t += 600
        cbraw = tx([("00" * 32, 0xffffffff, push(f"{tag}-{h}".encode()))], [(50 * 10**8, p2pkh(f"{tag}-cb-{h}"))])
        txs = []
        if i > 1:
            if unspent:
                ch, cid, cv, cspk = unspent[(i * 7 + len(tag)) % len(unspent)]
                unspent.remove((ch, cid, cv, cspk))
            else:
                ch, cid, cv, cspk = own.pop(0)
            a = tx([(cid, 0, push(f"{tag}-sig-{h}".encode()))], [(cv - 10**8, p2pkh(f"{tag}-a-{h}")), (10**8, p2pkh(f"{tag}-b-{h}"))])
            b = tx([(txid(a), 0, push(f"{tag}-sig2-{h}".encode()))], [(cv - 2 * 10**8, p2pkh(f"{tag}-c-{h}"))])
            txs = [{"raw": a.hex(), "prevouts": [{"value": cv, "spk": cspk.hex(), "coin_height": ch, "coin_is_coinbase": 1, "coin_mtp": mtp(ch - 1)}]},
                   {"raw": b.hex(), "prevouts": [{"value": cv - 10**8, "spk": p2pkh(f"{tag}-a-{h}").hex(), "coin_height": h, "coin_is_coinbase": 0, "coin_mtp": mtp(h - 1)}]}]
        j = {"height": h, "version": 1, "time": t, "bits": bits, "nonce": i, "prev": prev,
             "merkle": hashlib.sha256(f"{tag}-merkle-{h}".encode()).hexdigest(), "coinbase_hex": cbraw.hex(),
             "recent_times": [times[k] for k in range(max(0, h - 11), h)], "epoch_start": times[0],
             "prev_time": times[h - 1], "prev_bits": bits, "txs": txs}
        times[h] = t
        prev = header_hash(j)
        own.append((h, txid(cbraw), 50 * 10**8, p2pkh(f"{tag}-cb-{h}")))
        json.dump(j, open(out / f"block_{h}.json", "w"))
    print(f"branch {tag}: blocks {F + 1}..{F + L} from mainnet {F}, tip {prev}")

if __name__ == "__main__":
    main()
