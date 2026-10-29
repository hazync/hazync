"""efcheck.py <ours.hints> <published.hints>: decode both (hintsfile v0, Elias-Fano per height) and check
that for every height <= our stop height, the published file's unspent indices (at ITS stop height, later
than ours) are a SUBSET of ours: a coin unspent at 970,000 was unspent at every earlier height after its
creation, and both files must number outputs the same way for the indices to line up.
"""
import sys

def compact(f):
    b = f.read(1)[0]
    if b < 0xFD: return b
    if b == 0xFD: return int.from_bytes(f.read(2), "little")
    if b == 0xFE: return int.from_bytes(f.read(4), "little")
    raise ValueError("unexpected 0xFF compact size")

def ef(f):
    n = compact(f)
    if n == 0: return []
    m = compact(f)
    l = ((m + 1) // n).bit_length() - 1
    low = f.read((n * l + 7) // 8)
    high = f.read((n + (m >> l) + 7) // 8)
    out, lb, hb, prefix = [], 0, 0, 0
    for _ in range(n):
        v = 0
        for s in range(l):
            v |= ((low[lb // 8] >> (lb % 8)) & 1) << s
            lb += 1
        while True:
            bit = (high[hb // 8] >> (hb % 8)) & 1
            hb += 1
            if bit: break
            prefix += 1
        out.append((prefix << l) | v)
    return out

def load(path, upto):
    f = open(path, "rb")
    assert f.read(4) == b"UTXO" and f.read(1) == b"\x00", "not a hintsfile v0"
    stop = int.from_bytes(f.read(4), "little")
    return stop, {h: ef(f) for h in range(1, min(stop, upto) + 1)}

ours_stop, ours = load(sys.argv[1], 10**9)
pub_stop, pub = load(sys.argv[2], ours_stop)
bad = [h for h in range(1, ours_stop + 1) if not set(pub[h]) <= set(ours[h])]
n_ours = sum(len(v) for v in ours.values()); n_pub = sum(len(v) for v in pub.values())
print(f"ours: stop {ours_stop}, {n_ours} unspent; published: stop {pub_stop}, {n_pub} of those heights' coins still unspent")
print("SUBSET_OK" if not bad else f"SUBSET_FAIL at heights {bad[:10]}")
diff = [(h, sorted(set(ours[h]) - set(pub[h]))) for h in range(1, ours_stop + 1) if set(ours[h]) != set(pub[h])]
print(f"heights where a coin unspent at {ours_stop} was spent by {pub_stop}: {len(diff)}  e.g. {diff[:3]}")
