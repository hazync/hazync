#!/usr/bin/env python3
"""E1 estimate: what an outpoint-keyed tree of live coins (for proofs of ABSENCE) costs per block, before building it.

Model: a compressed sparse Merkle tree (Jellyfish/Patricia-style: a key's path stops where its prefix becomes unique),
n live keys, a block's k updates (each spend deletes a key, each surviving output inserts one) applied as one batch.
The guest checks the old root and computes the new one over the union of touched paths, so the hash count is about
2 x (distinct nodes on the touched paths). Keys are uniform (hashes of outpoints), so the touched nodes are simulated
with random keys at the real depth (log2 n). Cycles per hash: the coin log's measured 100-525 per compression.
usage: estimate.py <block.json> [live coins, default 180e6]
"""
import json, math, random, sys

j = json.load(open(sys.argv[1]))
n = float(sys.argv[2]) if len(sys.argv) > 2 else 180e6
txs = j["txs"]
ins = sum(len(t["prevouts"]) for t in txs[1:])
# outputs: count from raw (rough: parse output count) -- use the coin log rule's appends if present, else 1.4 x inputs
def nout(raw):
    b = bytes.fromhex(raw); o = 4
    if b[4] == 0 and b[5] == 1: o += 2
    def vi():
        nonlocal o
        f = b[o]; o += 1
        if f < 0xfd: return f
        k = {0xfd: 2, 0xfe: 4, 0xff: 8}[f]; v = int.from_bytes(b[o:o+k], "little"); o += k; return v
    for _ in range(vi()):
        o += 36; l = vi(); o += l + 4
    return vi()
outs = sum(nout(t["raw"]) for t in txs[1:]) + 2
k = ins + outs
depth = math.log2(n)
random.seed(1)
D = int(depth) + 1
keys = [random.getrandbits(64) for _ in range(k)]
distinct = 0
for lvl in range(1, D + 1):
    distinct += len({x >> (64 - lvl) for x in keys})
# compressed: below the level where a key's prefix is unique among n keys (~depth), nothing; we cut at D
hashes = 2 * distinct
print(f"block {j['height']}: {ins} inputs, {outs} outputs, k = {k} updates, n = {n:.3g} live keys, depth ~ {depth:.1f}")
print(f"distinct touched nodes {distinct:,} ({distinct / k:.1f} per update); hashes ~ {hashes:,}")
for c in (100, 250, 525):
    print(f"  at {c} cycles/hash: {hashes * c / 1e6:.0f} M cycles")
