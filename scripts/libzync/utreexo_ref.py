#!/usr/bin/env python3
"""utreexo_ref.py <block json dir> <H> [state.json] -- an INDEPENDENT implementation of libzync's UTXO
accumulator (docs/libzync/UTREEXO_FORMAT.md), written from the spec, not from the guest.

Reads block_<h>.json for h = 1..H (the host's block files: header fields, coinbase_hex, txs[].raw), works out
which outputs are unspent at H by following every spend (no hints file), builds the append-only forest of those
coins in height order, each block's sorted by (txid, vout), and prints num_leaves and the roots. Each block's creation MTP is computed here from the
header timestamps of blocks 0..h-1 (genesis by its hash, from the table below; the others from the files, which
must link by hash); the host's own `recent_times` field is only cross-checked against it, never used. With a ChainState JSON (a sweep's state_<H>.json) it compares against the
guest's committed utxo_leaves / utxo_roots and exits 1 on any difference.

Python standard library only.
"""
import os, hashlib, json, sys

TAG_LEAF, TAG_NODE = b"\x00", b"\x01"
MAX_SCRIPT_SIZE = 10_000
# Core's IsBIP30Unspendable (mainnet): these two coinbases' outputs are never in the UTXO set.
BIP30_UNSPENDABLE = {91722: "00000000000271a2dc26e7667f8419f2e15416dc6955e5a6c6cdf3f2574dd08e",
                     91812: "00000000000af0aed4792b1acee3d966af36cf5def14935db8de83d6f9306f2f"}


# Genesis block hash (display order) -> timestamp, for the networks this is used on.
GENESIS_TIME = {"000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f": 1231006505,   # mainnet
                "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6": 1598918400,   # signet
                "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206": 1296688602}   # regtest


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


def leaf(txid, vout, value, spk, height, cb, coin_mtp, tracked=False):
    return sha256(TAG_LEAF + txid + vout.to_bytes(4, "little") + value.to_bytes(8, "little")
                  + len(spk).to_bytes(4, "little") + spk + height.to_bytes(4, "little")
                  + bytes([(1 if cb else 0) | (2 if tracked else 0)]) + coin_mtp.to_bytes(4, "little"))


# BIP34 activation by genesis block (Core's chainparams; regtest as the rig runs it, -testactivationheight=bip34@110).
BIP34_HEIGHT = {"000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f": 227931,
                "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6": 1,
                "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206": 110}
REPEAT_HORIZON = 100_000_000


def indicated_height(raw):
    """The height a coinbase's scriptSig reads as under BIP34's encoding (a minimal push of a number), else 0."""
    i = 4 + (2 if raw[4] == 0 and raw[5] == 1 else 0)
    _, i = varint(raw, i)
    i += 36
    sl, i = varint(raw, i)
    ss = raw[i:i + sl]
    if not ss:
        return 0
    op = ss[0]
    if 0x51 <= op <= 0x60:
        v, enc = op - 0x50, bytes([op])
    elif 1 <= op <= 5 and len(ss) > op:
        v = int.from_bytes(ss[1:1 + op], "little")
        enc = None
    else:
        return 0
    if enc is None:   # the push must be the minimal CScriptNum encoding of v (CScript() << v)
        b = v.to_bytes((v.bit_length() + 7) // 8, "little") if v else b""
        if b and b[-1] & 0x80:
            b += b"\x00"
        enc = (bytes([0x51 + v - 1]) if 1 <= v <= 16 else bytes([len(b)]) + b) if v else b"\x00"
    return v if ss[:len(enc)] == enc else 0


class Tracked:
    """The BIP30 tree's tracked transactions (coinbase-smt/src/bip30.rs): a coinbase mined before BIP34 whose script
    reads as a later height below REPEAT_HORIZON, the repeat of a tracked coinbase, and any transaction whose inputs
    are all tracked coins. Their outputs carry leaf flag bit 1 (UTREEXO_FORMAT.md §2)."""
    def __init__(self, genesis):
        self.bip34 = BIP34_HEIGHT[genesis]
        self.coins, self.txids = set(), set()

    def tx(self, h, k, raw, txid, ins):
        if k == 0:
            ind = indicated_height(raw)
            t = (h < self.bip34 and h < ind < REPEAT_HORIZON) or txid in self.txids
        else:
            t = len(ins) > 0 and all(op in self.coins for op in ins)
        if t:
            self.txids.add(txid)
        return t

    def created(self, op):
        self.coins.add(op)


def spendable(spk):
    return not (len(spk) > 0 and spk[0] == 0x6A) and len(spk) <= MAX_SCRIPT_SIZE


def main():
    d, H = sys.argv[1], int(sys.argv[2])
    created = []          # (sort key, outpoint, leaf, value); key = (height, txid internal bytes, vout)
    meta = {}             # outpoint -> (height, coinbase, creation MTP), to check the block files' prevouts
    spent = set()
    times = []            # header timestamps of blocks 0..h-1
    prev_hash = None
    for h in range(1, H + 1):
        j = json.load(open(f"{d}/block_{h}.json"))
        if h == 1:
            assert j["prev"] in GENESIS_TIME, f"block 1's parent {j['prev']} is not a known genesis"
            trk = Tracked(j["prev"])
            times.append(GENESIS_TIME[j["prev"]])
        else:
            assert j["prev"] == prev_hash, f"block {h} does not link to block {h - 1}"
        prev_hash = header_hash(j)
        m = mtp(times[-11:])         # MTP of the parent: the creation MTP every leaf of this block commits
        assert sorted(j["recent_times"]) == sorted(times[-11:]), f"block {h}: file's recent_times differ from the headers"
        times.append(j["time"])
        bip30 = h in BIP30_UNSPENDABLE and BIP30_UNSPENDABLE[h] == header_hash(j)
        raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
        for k, raw in enumerate(raws):
            txid, ins, outs = parse_tx(raw)
            tr = trk.tx(h, k, raw, txid, ins)
            if k > 0:
                for (pt, pv), p in zip(ins, j["txs"][k - 1]["prevouts"]):
                    spent.add((pt, pv))
                    # Self-check: the block file's prevout metadata must equal what this script derived.
                    if (pt, pv) in meta:
                        got = (p["coin_height"], p["coin_is_coinbase"] & 1 == 1, p["coin_mtp"])
                        assert meta[(pt, pv)] == got, f"block {h}: prevout {pt[::-1].hex()}:{pv} {meta[(pt, pv)]} != file {got}"
            if k == 0 and bip30:
                continue
            for v, (value, spk) in enumerate(outs):
                meta[(txid, v)] = (h, k == 0, m)
                if tr:
                    trk.created((txid, v))
                if spendable(spk):
                    created.append(((h, txid, v), (txid, v), leaf(txid, v, value, spk, h, k == 0, m, tr), value))
    # Within a block, survivors are appended sorted by (txid internal bytes, vout): Core's own UTXO key order, so
    # a Core `dumptxoutset` (which carries each coin's height) gives the order with no block data.
    live = sorted(c for c in created if c[1] not in spent)
    survivors = [l for _, _, l, _ in live]
    utxo_value = sum(v for _, _, _, v in live)
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
    if os.environ.get("STATES_OUT"):
        # The forest after each height 0..H, for block_forest_ref.py: the survivors (to H) created up to that height.
        st, out, val, k = [], [{"h": 0, "leaves": 0, "value": 0, "roots": []}], 0, 0
        for h in range(1, H + 1):
            while k < len(live) and live[k][0][0] == h:
                node, hgt = live[k][2], 0
                val += live[k][3]
                while hgt < len(st) and st[hgt] is not None:
                    node = sha256(TAG_NODE + st[hgt] + node)
                    st[hgt] = None
                    hgt += 1
                if hgt == len(st):
                    st.append(None)
                st[hgt] = node
                k += 1
            r = list(st)
            while r and r[-1] is None:
                r.pop()
            out.append({"h": h, "leaves": k, "value": val, "roots": [x.hex() if x else None for x in r]})
        with open(os.environ["STATES_OUT"], "w") as f:
            f.writelines(json.dumps(x) + "\n" for x in out)
    print(f"H={H}: {len(survivors)} leaves, {utxo_value} sat, roots:")
    for hgt, r in enumerate(roots):
        print(f"  [{hgt}] {r.hex() if r else '-'}")
    if len(sys.argv) > 3:
        st = json.load(open(sys.argv[3]))
        g = [bytes(x) if x is not None else None for x in st["utxo_roots"]]
        while g and g[-1] is None:
            g.pop()
        ok = st["utxo_leaves"] == len(survivors) and g == roots and st.get("utxo_value") == utxo_value
        print("REF_MATCH: guest state equals this implementation" if ok else
              f"REF_MISMATCH: guest {st['utxo_leaves']} leaves / {len(g)} roots / {st.get('utxo_value')} sat")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
