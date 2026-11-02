#!/usr/bin/env python3
"""coinlog_ref.py <block json dir> <H> [state.json]: the coin log (libzync/src/coinlog.rs), written from its doc comment
alone and sharing no code with the guest or the host. Python standard library only; SHA-256's compression function is
implemented here because hashlib does not expose it.

Every coin that outlives the block creating it is appended in block order (transactions in block order, outputs in
vout order, unspendable outputs and the two BIP30-unspendable coinbases skipped, as UTREEXO_FORMAT.md §3); a spend of a
coin from an earlier block flips its leaf to spent(leaf, spender), spender = height u32 LE ‖ position of the spending
transaction in its block u32 LE (coinbase 0) ‖ input index u32 LE ‖ 20 zero bytes. Roots: one perfect tree per set bit of the leaf count, oldest
and largest first. With a state, compares utxo_roots, utxo_leaves (= every coin ever appended) and utxo_value (= the
live coins' total). Prints COINLOG_REF_MATCH / COINLOG_REF_MISMATCH and exits 0 / 1.
With STATES_OUT=<file>, writes the commitment after every height 0..H, one JSON line each, for block_forest_ref.py.
"""
import hashlib, json, os, struct, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from utreexo_ref import parse_tx, header_hash, mtp, spendable, leaf, BIP30_UNSPENDABLE, GENESIS_TIME, Tracked

K = [0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
     0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
     0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
     0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
     0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
     0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
     0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2]
M = 0xFFFFFFFF


def rotr(x, n):
    return ((x >> n) | (x << (32 - n))) & M


def compress(state, block):
    """FIPS 180-4 SHA-256 compression of one 64-byte block onto 8 state words."""
    w = list(struct.unpack(">16I", block))
    for i in range(16, 64):
        s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
        s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
        w.append((w[i - 16] + s0 + w[i - 7] + s1) & M)
    a, b, c, d, e, f, g, h = state
    for i in range(64):
        t1 = (h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) & M
        t2 = ((rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) & M
        h, g, f, e, d, c, b, a = g, f, e, (d + t1) & M, c, b, a, (t1 + t2) & M
    return [(x + y) & M for x, y in zip(state, [a, b, c, d, e, f, g, h])]


def iv(tag):
    return list(struct.unpack(">8I", hashlib.sha256(tag).digest()))


IV_NODE = [iv(b"libzync-coinlog-node" + bytes([l])) for l in range(64)]
IV_SPENT = iv(b"libzync-coinlog-spent")


def comp(ivw, a, b):
    return struct.pack(">8I", *compress(ivw, a + b))


def parent(l, a, b):
    return comp(IV_NODE[l], a, b)


def spent(c, height, tx, vin):
    return comp(IV_SPENT, c, struct.pack("<III", height, tx, vin) + bytes(20))


def roots(leaves):
    n, out, start = len(leaves), {}, 0
    for l in range(63, -1, -1):
        if n >> l & 1:
            level = leaves[start:start + (1 << l)]
            for k in range(1, l + 1):
                level = [parent(k, level[2 * i], level[2 * i + 1]) for i in range(len(level) // 2)]
            out[l] = level[0]
            start += 1 << l
    top = max(out) + 1 if out else 0
    return [out.get(l) for l in range(top)]


class Inc:
    """The same forest kept incrementally (node lists per level), for the per-height states; checked against roots()."""
    def __init__(self):
        self.lv = [[]]

    def push(self, x):
        self.lv[0].append(x)
        l = 0
        while len(self.lv[l]) % 2 == 0:
            if len(self.lv) == l + 1:
                self.lv.append([])
            self.lv[l + 1].append(parent(l + 1, self.lv[l][-2], self.lv[l][-1]))
            l += 1

    def set(self, p, x):
        self.lv[0][p] = x
        for l in range(len(self.lv) - 1):
            j = p // 2
            if j >= len(self.lv[l + 1]):
                break
            self.lv[l + 1][j] = parent(l + 1, self.lv[l][2 * j], self.lv[l][2 * j + 1])
            p = j

    def roots(self):
        n, out, start = len(self.lv[0]), [], 0
        for l in range(63, -1, -1):
            if n >> l & 1:
                out += [None] * (l + 1 - len(out))
                out[l] = self.lv[l][start >> l]
                start += 1 << l
        return out


def main():
    d, H = sys.argv[1], int(sys.argv[2])
    log, pos, value = [], {}, {}     # leaf values; outpoint -> position of an unspent coin; outpoint -> amount
    times, prev_hash = [], None
    states = [{"h": 0, "leaves": 0, "value": 0, "roots": []}]
    inc = Inc()
    for h in range(1, H + 1):
        j = json.load(open(f"{d}/block_{h}.json"))
        if h == 1:
            times.append(GENESIS_TIME[j["prev"]])
            trk = Tracked(j["prev"])
        else:
            assert j["prev"] == prev_hash, f"block {h} does not link to block {h - 1}"
        prev_hash = header_hash(j)
        m = mtp(times[-11:])
        times.append(j["time"])
        bip30 = h in BIP30_UNSPENDABLE and BIP30_UNSPENDABLE[h] == header_hash(j)
        raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
        created, spent_here = [], []   # (outpoint, leaf, amount) in block order; (outpoint, tx position, input index)
        for k, raw in enumerate(raws):
            txid, ins, outs = parse_tx(raw)
            tr = trk.tx(h, k, raw, txid, ins)
            if k > 0:
                spent_here += [(op, k, i) for i, op in enumerate(ins)]
            if k == 0 and bip30:
                continue
            for v, (val, spk) in enumerate(outs):
                if tr:
                    trk.created((txid, v))
                if spendable(spk):
                    created.append(((txid, v), leaf(txid, v, val, spk, h, k == 0, m, tr), val))
        made = {op for op, _, _ in created}
        for op, k, i in spent_here:
            if op in made:
                made.discard(op)            # created and spent in this block: never enters the log
            else:
                p = pos.pop(op)             # a coin from an earlier block: flip it
                log[p] = spent(log[p], h, k, i)
                inc.set(p, log[p])
                value.pop(op)
        for op, lf, val in created:
            if op in made:
                pos[op] = len(log)
                log.append(lf)
                inc.push(lf)
                value[op] = val
        if os.environ.get("STATES_OUT"):
            states.append({"h": h, "leaves": len(log), "value": sum(value.values()),
                           "roots": [x.hex() if x else None for x in inc.roots()]})
    if os.environ.get("STATES_OUT"):
        with open(os.environ["STATES_OUT"], "w") as f:
            f.writelines(json.dumps(x) + "\n" for x in states)
    r = roots(log)
    assert inc.roots() == r, "incremental forest differs from the definition"
    total = sum(value.values())
    print(f"H={H}: {len(log)} leaves appended, {len(pos)} live coins, {total} sat, {sum(x is not None for x in r)} roots")
    if len(sys.argv) > 3:
        st = json.load(open(sys.argv[3]))
        g = [bytes(x) if x is not None else None for x in st["utxo_roots"]]
        while g and g[-1] is None:
            g.pop()
        ok = g == r and st["utxo_leaves"] == len(log) and st.get("utxo_value") == total
        print("COINLOG_REF_MATCH: the proof's coin log equals this implementation" if ok else
              f"COINLOG_REF_MISMATCH: proof {st['utxo_leaves']} leaves / {st.get('utxo_value')} sat / roots equal {g == r}")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
