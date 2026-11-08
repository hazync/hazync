#!/usr/bin/env python3
# E1 option B/C input: coin lifetimes over a window of consecutive mainnet blocks. Read-only: each block is fetched raw
# with bitcoin-cli on the bridge node over ssh and parsed here; only 8-byte outpoint ids are kept.
# Per block: created = spendable outputs (not OP_RETURN), spent = non-coinbase inputs. Writes <out>.json:
# per block [height, created ids, spent ids] as hex-free ints, then summary.py-style stats printed.
# usage: lifetimes.py <first height> <count> <out prefix> [workers]
import hashlib, json, subprocess, sys
from multiprocessing import Pool

def rpc(*a):
    return subprocess.run(['ssh', 'hazync-coord', 'bitcoin-cli', *map(str, a)], capture_output=True, text=True, check=True, timeout=600).stdout.strip()

def varint(b, o):
    f = b[o]
    if f < 0xfd: return f, o + 1
    n = {0xfd: 2, 0xfe: 4, 0xff: 8}[f]
    return int.from_bytes(b[o + 1:o + 1 + n], 'little'), o + 1 + n

def oid(txid, vout):
    return int.from_bytes(hashlib.sha256(txid + vout.to_bytes(4, 'little')).digest()[:8], 'little')

def block(h):
    b = bytes.fromhex(rpc('getblock', rpc('getblockhash', h), 0))
    n, o = varint(b, 80)
    created, spent = [], []
    for _ in range(n):
        s = o
        o += 4
        seg = b[o] == 0 and b[o + 1] == 1
        if seg: o += 2
        body0 = o
        nin, o = varint(b, o)
        ins = []
        for _ in range(nin):
            ins.append((b[o:o + 32], int.from_bytes(b[o + 32:o + 36], 'little')))
            o += 36
            l, o = varint(b, o); o += l + 4
        nout, o = varint(b, o)
        outs = []
        for _ in range(nout):
            o += 8
            l, o = varint(b, o); outs.append(b[o:o + l]); o += l
        body1 = o
        if seg:
            for _ in range(nin):
                k, o = varint(b, o)
                for _ in range(k):
                    l, o = varint(b, o); o += l
        lt = b[o:o + 4]; o += 4
        txid = hashlib.sha256(hashlib.sha256(b[s:s + 4] + b[body0:body1] + lt).digest()).digest()
        for t, v in ins:
            if t != b'\0' * 32 or v != 0xffffffff: spent.append(oid(t, v))
        for v, spk in enumerate(outs):
            if not (spk[:1] == b'\x6a' or len(spk) > 10000): created.append(oid(txid, v))
    assert o == len(b), h
    return h, created, spent

if __name__ == '__main__':
    first, count, out = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
    with Pool(int(sys.argv[4]) if len(sys.argv) > 4 else 8) as p:
        rows = sorted(p.imap_unordered(block, range(first, first + count)))
    json.dump(rows, open(out + '.json', 'w'))
    print('LIFETIMES_FETCHED', out, len(rows))
