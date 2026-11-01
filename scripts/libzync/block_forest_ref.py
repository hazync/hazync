#!/usr/bin/env python3
"""block_forest_ref.py: the block forest and lineage digest, written from docs/libzync/HANDOVER.md alone and sharing
no code with the guest or the verifier. Python standard library only.

  block_forest_ref.py vectors                   print the test vectors that swiftsync-verify's unit tests pin
  block_forest_ref.py check <hashes> <state>    <hashes>: one block hash per line, display order, height 0 first.
                                                <state>: a ChainState JSON at the last height. Compares the proof's
                                                block_roots and block_leaves with this file's. Exit 1 on any difference.
"""
import hashlib, json, struct, sys

TAG_NODE, TAG_BLOCK_LEAF, TAG_LINEAGE = 0x01, 0x02, 0x03


def sha256(b):
    return hashlib.sha256(b).digest()


def leaf(block_hash, height):
    return sha256(bytes([TAG_BLOCK_LEAF]) + block_hash + struct.pack("<I", height))


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
    """handovers: (image id as 8 u32 words, first height), oldest first."""
    d = bytes(32)
    for words, first in handovers:
        d = sha256(bytes([TAG_LINEAGE]) + d + struct.pack("<8I", *words) + struct.pack("<I", first))
    return d


def vectors():
    # Deterministic made-up block hashes: b_i = SHA-256 of the height as u32 LE.
    hs = [sha256(struct.pack("<I", i)) for i in range(11)]
    print("leaf(b_0, 0)      ", leaf(hs[0], 0).hex())
    for i, r in enumerate(roots(hs)):
        print(f"root[{i}] of 11    ", r.hex() if r else "None")
    for i, p in enumerate(path(hs, 5)):
        print(f"path(5)[{i}]        ", p.hex())
    print("lineage(1 hop)    ", lineage([((1, 2, 3, 4, 5, 6, 7, 8), 974000)]).hex())
    print("lineage(2 hops)   ", lineage([((1, 2, 3, 4, 5, 6, 7, 8), 974000), ((9, 9, 9, 9, 9, 9, 9, 9), 980000)]).hex())


def check(hashes_path, state_path):
    hs = [bytes.fromhex(l.strip())[::-1] for l in open(hashes_path) if l.strip()]
    st = json.load(open(state_path))
    want = [bytes(r) if r else None for r in st["block_roots"]]
    got = roots(hs)
    print(f"blocks {len(hs)}, proof height {st['height']}, proof leaves {st['block_leaves']}")
    ok = st["block_leaves"] == len(hs) == st["height"] + 1 and want == got
    print("BLOCK_FOREST_OK" if ok else f"BLOCK_FOREST_MISMATCH\n proof {[r.hex() if r else None for r in want]}\n ours  {[r.hex() if r else None for r in got]}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if sys.argv[1:2] == ["vectors"]:
        vectors()
    elif sys.argv[1:2] == ["check"] and len(sys.argv) == 4:
        check(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
