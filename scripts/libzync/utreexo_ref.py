#!/usr/bin/env python3
"""utreexo_ref.py <block json dir> <H> [state.json] -- an INDEPENDENT implementation of libzync's UTXO
accumulator (docs/libzync/UTREEXO_FORMAT.md), written from the spec, not from the guest.

Reads block_<h>.json for h = 1..H (the host's block files: header fields, coinbase_hex, txs[].raw,
recent_times = the 11 prior timestamps), works out which outputs are unspent at H by following every
spend (no hints file), builds the append-only forest of those coins in creation order, and prints
num_leaves and the roots. With a ChainState JSON (a sweep's state_<H>.json) it compares against the
guest's committed utxo_leaves / utxo_roots and exits 1 on any difference.

Python standard library only.
"""
import hashlib, json, sys

TAG_LEAF, TAG_NODE = b"\x00", b"\x01"
MAX_SCRIPT_SIZE = 10_000
# Core's IsBIP30Unspendable (mainnet): these two coinbases' outputs are never in the UTXO set.
BIP30_UNSPENDABLE = {91722: "00000000000271a2dc26e7667f8419f2e15416dc6955e5a6c6cdf3f2574dd08e",
                     91812: "00000000000af0aed4792b1acee3d966af36cf5def14935db8de83d6f9306f2f"}


def sha256(b):
    return hashlib.sha256(b).digest()


def dsha(b):
    return sha256(sha256(b))


def varint(b, i):
    x = b[i]
    if x < 0xFD:
        return x, i + 1
    n = {0xFD: 2, 0xFE: 4, 0xFF: 8}[x]
    return int.from_bytes(b[i + 1:i + 1 + n], "little"), i + 1 + n


def parse_tx(raw):
    """(txid internal order, [(prev txid, prev vout)], [(value, spk)])."""
    i = 4
    segwit = raw[4] == 0 and raw[5] == 1
    if segwit:
        i += 2
    n, i = varint(raw, i)
    ins = []
    for _ in range(n):
        ins.append((raw[i:i + 32], int.from_bytes(raw[i + 32:i + 36], "little")))
        i += 36
        sl, i = varint(raw, i)
        i += sl + 4
    n, i = varint(raw, i)
    outs = []
    for _ in range(n):
        v = int.from_bytes(raw[i:i + 8], "little")
        sl, i = varint(raw, i + 8)
        outs.append((v, raw[i:i + sl]))
        i += sl
    end_io = i
    if segwit:
        for _ in ins:
            k, i = varint(raw, i)
            for _ in range(k):
                l, i = varint(raw, i)
                i += l
    stripped = raw[:4] + raw[6:end_io] + raw[i:i + 4] if segwit else raw
    return dsha(stripped), ins, outs


def header_hash(j):
    h = (j["version"].to_bytes(4, "little", signed=True) + bytes.fromhex(j["prev"])[::-1]
         + bytes.fromhex(j["merkle"])[::-1] + j["time"].to_bytes(4, "little")
         + j["bits"].to_bytes(4, "little") + j["nonce"].to_bytes(4, "little"))
    return dsha(h)[::-1].hex()


def mtp(times):
    s = sorted(times)
    return s[len(s) // 2]


def leaf(txid, vout, value, spk, height, cb, coin_mtp):
    return sha256(TAG_LEAF + txid + vout.to_bytes(4, "little") + value.to_bytes(8, "little")
                  + len(spk).to_bytes(4, "little") + spk + height.to_bytes(4, "little")
                  + bytes([1 if cb else 0]) + coin_mtp.to_bytes(4, "little"))


def spendable(spk):
    return not (len(spk) > 0 and spk[0] == 0x6A) and len(spk) <= MAX_SCRIPT_SIZE


def main():
    d, H = sys.argv[1], int(sys.argv[2])
    created = []          # (outpoint, leaf), creation order
    meta = {}             # outpoint -> (height, coinbase, creation MTP), to check the block files' prevouts
    spent = set()
    for h in range(1, H + 1):
        j = json.load(open(f"{d}/block_{h}.json"))
        m = mtp(j["recent_times"])   # MTP of the parent: the creation MTP every leaf of this block commits
        bip30 = BIP30_UNSPENDABLE.get(h) == header_hash(j)
        raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
        for k, raw in enumerate(raws):
            txid, ins, outs = parse_tx(raw)
            if k > 0:
                for (pt, pv), p in zip(ins, j["txs"][k - 1]["prevouts"]):
                    spent.add((pt, pv))
                    # Self-check: the block file's prevout metadata must equal what this script derived.
                    if (pt, pv) in meta:
                        got = (p["coin_height"], p["coin_is_coinbase"] == 1, p["coin_mtp"])
                        assert meta[(pt, pv)] == got, f"block {h}: prevout {pt[::-1].hex()}:{pv} {meta[(pt, pv)]} != file {got}"
            if k == 0 and bip30:
                continue
            for v, (value, spk) in enumerate(outs):
                meta[(txid, v)] = (h, k == 0, m)
                if spendable(spk):
                    created.append(((txid, v), leaf(txid, v, value, spk, h, k == 0, m)))
    survivors = [l for op, l in created if op not in spent]
    roots = []            # roots[h] = root of the perfect tree of 2^h leaves, or None
    for node in survivors:
        hgt = 0
        while hgt < len(roots) and roots[hgt] is not None:
            node = sha256(TAG_NODE + roots[hgt] + node)
            roots[hgt] = None
            hgt += 1
        if hgt == len(roots):
            roots.append(None)
        roots[hgt] = node
    while roots and roots[-1] is None:
        roots.pop()
    print(f"H={H}: {len(survivors)} leaves, roots:")
    for hgt, r in enumerate(roots):
        print(f"  [{hgt}] {r.hex() if r else '-'}")
    if len(sys.argv) > 3:
        st = json.load(open(sys.argv[3]))
        g = [bytes(x) if x is not None else None for x in st["utxo_roots"]]
        while g and g[-1] is None:
            g.pop()
        ok = st["utxo_leaves"] == len(survivors) and g == roots
        print("REF_MATCH: guest state equals this implementation" if ok else
              f"REF_MISMATCH: guest {st['utxo_leaves']} leaves / {len(g)} roots")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
