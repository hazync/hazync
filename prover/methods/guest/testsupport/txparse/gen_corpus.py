#!/usr/bin/env python3
"""The corpus of the transaction-parser differential test (test-txparse.sh), deterministic (fixed seed).

  gen_corpus.py <repo> <core v28.0 src/test/data> > corpus.txt     one line per case: "T <hex>" or "P <hex>"

1. real mainnet transactions from the repo's block fixtures (prover/block_*.json, prover/testdata/boundaries/*.json):
   every transaction and coinbase, each transaction's stripped form, and each transaction's prevouts blob (a
   serialised std::vector<CTxOut>, as the guest receives it);
2. Core v28.0's tx_valid.json / tx_invalid.json transactions;
3. mutations of a sample of those: truncations, trailing bytes, non-canonical and oversized compact sizes (counts,
   script lengths, witness counts), witness marker / flag variants (flag 0, 2, 3, 0x81, 0xff; the marker with every
   stack empty; a legacy transaction given a marker), zero-input forms, huge script lengths, byte flips.
The field locator below is only used to aim mutations; it is never the reference (Core's harness is).
"""
import glob, json, os, random, sys

MAX_SIZE = 0x02000000


def cs(n):
    if n < 0xfd: return bytes([n])
    if n <= 0xffff: return b"\xfd" + n.to_bytes(2, "little")
    if n <= 0xffffffff: return b"\xfe" + n.to_bytes(4, "little")
    return b"\xff" + n.to_bytes(8, "little")


def rcs(b, o):
    f = b[o]
    if f < 0xfd: return f, o + 1
    n = {0xfd: 2, 0xfe: 4, 0xff: 8}[f]
    return int.from_bytes(b[o + 1:o + 1 + n], "little"), o + 1 + n


def locate(b):
    """(compact-size fields as (pos, end, value), witness flag position or None, witness ranges) of a well-formed tx."""
    fields, o, seg = [], 4, None
    if b[4] == 0 and b[5] == 1:
        seg, o = 5, 6
    nin, e = rcs(b, o); fields.append((o, e, nin, "nin")); o = e
    for _ in range(nin):
        o += 36
        l, e = rcs(b, o); fields.append((o, e, l, "sig")); o = e + l + 4
    nout, e = rcs(b, o); fields.append((o, e, nout, "nout")); o = e
    for _ in range(nout):
        o += 8
        l, e = rcs(b, o); fields.append((o, e, l, "spk")); o = e + l
    wit = None
    if seg is not None:
        w0 = o
        for _ in range(nin):
            k, e = rcs(b, o); fields.append((o, e, k, "witn")); o = e
            for _ in range(k):
                l, e = rcs(b, o); fields.append((o, e, l, "witl")); o = e + l
        wit = (w0, o)
    assert o + 4 == len(b), "locator out of step"
    return fields, seg, wit


def stripped(b, seg, wit):
    return b[:4] + b[6:wit[0]] + b[-4:]


def prevouts_blob(pv):
    out = cs(len(pv))
    for p in pv:
        s = bytes.fromhex(p["spk"])
        out += int(p["value"]).to_bytes(8, "little") + cs(len(s)) + s
    return out


def mut_cs(b, pos, end, val):
    """Every other encoding of one compact size, and oversize values in its place."""
    out = []
    for enc in (b"\xfd" + (val & 0xffff).to_bytes(2, "little"), b"\xfe" + (val & 0xffffffff).to_bytes(4, "little"),
                b"\xff" + val.to_bytes(8, "little")):
        if enc != b[pos:end]:
            out.append(b[:pos] + enc + b[end:])
    for big in (MAX_SIZE, MAX_SIZE + 1, len(b) - end + 1, 0xffff, 0xffffffff):
        out.append(b[:pos] + cs(big) + b[end:])
    out.append(b[:pos] + b"\xff" + (1 << 40).to_bytes(8, "little") + b[end:])
    return out


def mutations(b, rng, full):
    out = []
    try:
        fields, seg, wit = locate(b)
    except Exception:
        fields, seg, wit = [], None, None
    cuts = range(len(b)) if full else sorted(set(rng.randrange(len(b)) for _ in range(24)) | {p for f in fields for p in (f[0], f[1])})
    out += [b[:c] for c in cuts]
    out += [b + b"\x00", b + bytes(rng.randrange(256) for _ in range(5)), b + b"\x00\x01"]
    for (pos, end, val, kind) in fields[:12]:
        out += mut_cs(b, pos, end, val)
    if seg is not None:
        for flag in (0x00, 0x02, 0x03, 0x81, 0xff):
            out.append(b[:5] + bytes([flag]) + b[6:])
        nin = fields[0][2]
        out.append(b[:wit[0]] + b"\x00" * nin + b[-4:])                     # marker, every stack empty
        out.append(b[:4] + b[6:])                                           # marker dropped, witness left in
        s = stripped(b, seg, wit)
        out.append(s)
    else:
        nin = fields[0][2] if fields else 1
        if fields:
            body = b[4:-4]
            out.append(b[:4] + b"\x00\x01" + body + b"\x00" * nin + b[-4:])  # superfluous witness
            out.append(b[:4] + b"\x00\x01" + body + b"\x01\x01\x00" + b"\x00" * (nin - 1) + b[-4:])
    for _ in range(12):
        x = bytearray(b); i = rng.randrange(len(x)); x[i] ^= 1 << rng.randrange(8); out.append(bytes(x))
    return out


def main():
    repo, core_data = sys.argv[1], sys.argv[2]
    rng = random.Random(28112026)
    lines = []
    real, blobs, stripped_n = [], [], 0
    files = sorted(glob.glob(os.path.join(repo, "prover/block_*.json")) + glob.glob(os.path.join(repo, "prover/testdata/boundaries/*.json")))
    for f in files:
        d = json.load(open(f))
        real.append(bytes.fromhex(d["coinbase_hex"]))
        for t in d["txs"]:
            b = bytes.fromhex(t["raw"])
            real.append(b)
            blobs.append(prevouts_blob(t["prevouts"]))
    seen = set()
    real = [b for b in real if not (b in seen or seen.add(b))]
    for b in real:
        lines.append(("T", b))
        f, seg, wit = locate(b)
        if seg is not None:
            lines.append(("T", stripped(b, seg, wit))); stripped_n += 1
    for p in blobs:
        lines.append(("P", p))
    vec = []
    for name in ("tx_valid.json", "tx_invalid.json"):
        for e in json.load(open(os.path.join(core_data, name))):
            if isinstance(e, list) and len(e) >= 2 and isinstance(e[0], list) and isinstance(e[1], str):
                vec.append(bytes.fromhex(e[1]))
    for b in vec:
        lines.append(("T", b))
    # zero-input and bare forms
    v, lt = b"\x01\x00\x00\x00", b"\x00\x00\x00\x00"
    out1 = b"\x01" + (1).to_bytes(8, "little") + b"\x01\x51"
    for mid in (b"\x00\x00", b"\x00\x01\x00\x00", b"\x00\x01" + b"\x00" + out1, b"\x00" + out1, b"\x00\x02\x00\x00",
                b"\x00\x01\x01" + bytes(36) + b"\x00" + b"\xff" * 4 + out1 + b"\x00", b"", b"\x00", b"\x00\x01"):
        lines.append(("T", v + mid + lt))
    sample = rng.sample(real, min(400, len(real))) + vec[: 200]
    small = [b for b in sample if len(b) < 300][:40]
    n_mut = 0
    for b in sample:
        for m in mutations(b, rng, b in small):
            lines.append(("T", m)); n_mut += 1
    for p in rng.sample(blobs, min(300, len(blobs))):
        try:
            n, e = rcs(p, 0)
            first_len_pos = e + 8
            l, e2 = rcs(p, first_len_pos)
            ms = mut_cs(p, 0, e, n) + mut_cs(p, first_len_pos, e2, l) + [p[:c] for c in sorted(set(rng.randrange(len(p)) for _ in range(10)))] + [p + b"\x00"]
        except Exception:
            ms = []
        for m in ms:
            lines.append(("P", m)); n_mut += 1
    for k, b in lines:
        print(k, b.hex())
    print(f"corpus: {len(real)} real transactions ({stripped_n} also stripped), {len(blobs)} prevouts blobs, "
          f"{len(vec)} Core vectors, {n_mut} mutations, {len(lines)} lines", file=sys.stderr)


if __name__ == "__main__":
    main()
