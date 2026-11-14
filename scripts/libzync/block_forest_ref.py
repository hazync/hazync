#!/usr/bin/env python3
"""block_forest_ref.py: the block forest and lineage digest, written from docs/libzync/HANDOVER.md alone and sharing
no code with the guest or the verifier. Python standard library only.

  block_forest_ref.py vectors                   print the test vectors that swiftsync-verify's unit tests pin
  block_forest_ref.py check <hashes> <states> <state>
        <hashes>: one block hash per line, display order, height 0 first.
        <states>: the coin commitment after each block, one JSON object per line, height 0 first:
                  {"h": height, "leaves": n, "value": sat, "roots": [hex or null, ...]} (index = tree level), as
                  coinlog_ref.py / utreexo_ref.py write them with STATES_OUT=<file>.
        <state>:  a ChainState JSON at the last height. Compares the proof's block_roots and block_leaves with this
                  file's. Exit 1 on any difference.
"""
import hashlib, json, struct, sys

TAG_NODE, TAG_BLOCK_LEAF, TAG_LINEAGE, TAG_UTXO_STATE = 0x01, 0x02, 0x03, 0x04


def sha256(b):
    return hashlib.sha256(b).digest()


def utxo_state(leaves, value, roots):
    """The coin commitment after a block: roots[l] is the tree of 2^l leaves or None; present roots largest first."""
    return sha256(bytes([TAG_UTXO_STATE]) + struct.pack("<QQ", leaves, value) + b"".join(r for r in reversed(roots) if r))


def leaf(block, height):
    """block = (hash in internal byte order, utxo_state digest after it)."""
    return sha256(bytes([TAG_BLOCK_LEAF]) + block[0] + struct.pack("<I", height) + block[1])


def parent(left, right):
    return sha256(bytes([TAG_NODE]) + left + right)


def perfect_root(leaves):
    while len(leaves) > 1:
        leaves = [parent(leaves[i], leaves[i + 1]) for i in range(0, len(leaves), 2)]
    return leaves[0]


def trees(n):
    """(tree height, first leaf) for a forest of n leaves, largest tree first: the binary digits of n."""
    out, start = [], 0
    for h in range(63, -1, -1):
        if n >> h & 1:
            out.append((h, start))
            start += 1 << h
    return out


def roots(hashes):
    """roots[h] is the root of the tree of height h, or None. Built from the definition, not by appending."""
    ls = [leaf(b, i) for i, b in enumerate(hashes)]
    ts = trees(len(ls))
    out = [None] * (max((h for h, _ in ts), default=-1) + 1)
    for h, start in ts:
        out[h] = perfect_root(ls[start:start + (1 << h)])
    return out


def path(hashes, height):
    for h, start in trees(len(hashes)):
        if start <= height < start + (1 << h):
            level, pos, out = [leaf(hashes[i], i) for i in range(start, start + (1 << h))], height - start, []
            for _ in range(h):
                out.append(level[pos ^ 1])
                level = [parent(level[i], level[i + 1]) for i in range(0, len(level), 2)]
                pos >>= 1
            return out
    raise ValueError(height)


def lineage(handovers):
    """handovers: (image id as 8 u32 words, first height[, verifier]), oldest first. `verifier` is the predecessor's
    RISC Zero succinct verifier-parameter digest (32 bytes) for a cross-version handover; absent or all zero = the same
    RISC Zero, and then the step is exactly the 69-byte preimage it always was (docs/HANDOVER.md §3)."""
    d = bytes(32)
    for h in handovers:
        words, first = h[0], h[1]
        verifier = h[2] if len(h) > 2 else bytes(32)
        pre = bytes([TAG_LINEAGE]) + d + struct.pack("<8I", *words) + struct.pack("<I", first)
        if verifier != bytes(32):
            pre += verifier
        d = sha256(pre)
    return d


def vectors():
    # Made-up blocks: hash b_i = SHA-256 of the height as u32 LE; coin state utxo_state(i, 1000 i, [None, SHA-256(b_i)])
    # for odd i, utxo_state(i, 1000 i, []) for even i.
    hs = []
    for i in range(11):
        b = sha256(struct.pack("<I", i))
        hs.append((b, utxo_state(i, 1000 * i, [None, sha256(b)] if i % 2 else [])))
    print("utxo_state(0,0,[])", utxo_state(0, 0, []).hex())
    print("leaf(b_0, 0)      ", leaf(hs[0], 0).hex())
    for i, r in enumerate(roots(hs)):
        print(f"root[{i}] of 11    ", r.hex() if r else "None")
    for i, p in enumerate(path(hs, 5)):
        print(f"path(5)[{i}]        ", p.hex())
    print("lineage(1 hop)    ", lineage([((1, 2, 3, 4, 5, 6, 7, 8), 974000)]).hex())
    print("lineage(2 hops)   ", lineage([((1, 2, 3, 4, 5, 6, 7, 8), 974000), ((9, 9, 9, 9, 9, 9, 9, 9), 980000)]).hex())
    print("lineage(xver hop) ", lineage([((1, 2, 3, 4, 5, 6, 7, 8), 974000), ((9, 9, 9, 9, 9, 9, 9, 9), 980000, bytes(range(32)))]).hex())


def check(hashes_path, states_path, state_path):
    bh = [bytes.fromhex(l.strip())[::-1] for l in open(hashes_path) if l.strip()]
    sts = [json.loads(l) for l in open(states_path) if l.strip()]
    assert [x["h"] for x in sts] == list(range(len(sts))), "states file: one line per height from 0"
    st = json.load(open(state_path))
    n = st["height"] + 1
    assert len(bh) >= n and len(sts) >= n, f"need {n} hashes and states, have {len(bh)} and {len(sts)}"
    hs = [(bh[i], utxo_state(sts[i]["leaves"], sts[i]["value"], [bytes.fromhex(r) if r else None for r in sts[i]["roots"]]))
          for i in range(n)]
    want = [bytes(r) if r else None for r in st["block_roots"]]
    got = roots(hs)
    print(f"blocks {len(hs)}, proof height {st['height']}, proof leaves {st['block_leaves']}")
    ok = st["block_leaves"] == len(hs) == st["height"] + 1 and want == got
    print("BLOCK_FOREST_OK" if ok else f"BLOCK_FOREST_MISMATCH\n proof {[r.hex() if r else None for r in want]}\n ours  {[r.hex() if r else None for r in got]}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if sys.argv[1:2] == ["vectors"]:
        vectors()
    elif sys.argv[1:2] == ["check"] and len(sys.argv) == 5:
        check(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        sys.exit(__doc__)
