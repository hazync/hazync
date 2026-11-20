#!/usr/bin/env python3
"""pprof_sum.py <profile.pprof> [regex ...]: total cycles, the top self frames, and the INCLUSIVE cycles of every
frame whose name matches each regex (a sample counts once per regex, however many matching frames its stack has).
risc0 writes RISC0_PPROF_OUT as a gzipped pprof whose sample value is cycles. No dependencies: a minimal protobuf
reader for the fields used (Profile 2 sample, 4 location, 5 function, 6 string_table)."""
import gzip, re, sys
from collections import Counter


def varint(b, i):
    r = s = 0
    while True:
        x = b[i]; i += 1
        r |= (x & 0x7F) << s; s += 7
        if x < 0x80:
            return r, i


def fields(b):
    i = 0
    while i < len(b):
        k, i = varint(b, i)
        f, t = k >> 3, k & 7
        if t == 0:
            v, i = varint(b, i)
        elif t == 2:
            n, i = varint(b, i); v = b[i:i + n]; i += n
        elif t == 1:
            v = b[i:i + 8]; i += 8
        elif t == 5:
            v = b[i:i + 4]; i += 4
        else:
            raise ValueError(f"wire type {t}")
        yield f, t, v


def packed(v, t):
    if t == 0:
        return [v]
    out, i = [], 0
    while i < len(v):
        x, i = varint(v, i); out.append(x)
    return out


def load(path):
    raw = open(path, "rb").read()
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    samples, locs, funcs, strs = [], {}, {}, []
    for f, t, v in fields(raw):
        if f == 2:
            ids, vals = [], []
            for g, tt, vv in fields(v):
                if g == 1: ids += packed(vv, tt)
                elif g == 2: vals += packed(vv, tt)
            samples.append((ids, vals[0] if vals else 0))
        elif f == 4:
            lid, fn = 0, []
            for g, tt, vv in fields(v):
                if g == 1: lid = vv
                elif g == 4:
                    for h, _, x in fields(vv):
                        if h == 1: fn.append(x)
            locs[lid] = fn
        elif f == 5:
            fid = name = 0
            for g, tt, vv in fields(v):
                if g == 1: fid = vv
                elif g == 2: name = vv
            funcs[fid] = name
        elif f == 6:
            strs.append(v.decode("utf-8", "replace"))
    def names(ids):
        return [strs[funcs[fid]] for lid in ids for fid in locs.get(lid, [])]
    return [(names(ids), v) for ids, v in samples]


def main():
    prof = load(sys.argv[1])
    total = sum(v for _, v in prof)
    print(f"total {total}")
    self_c = Counter()
    for st, v in prof:
        self_c[st[0] if st else "?"] += v
    print("--- self top 15")
    for n, v in self_c.most_common(15):
        print(f"{v:12d} {100*v/total:5.1f}% {n[:110]}")
    if len(sys.argv) > 2:
        print("--- inclusive")
    for rx in sys.argv[2:]:
        r = re.compile(rx)
        inc = sum(v for st, v in prof if any(r.search(n) for n in st))
        print(f"{inc:12d} {100*inc/total:5.1f}% /{rx}/")


if __name__ == "__main__":
    main()
