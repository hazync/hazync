"""core_share.py <profile.pb> [...] — how much of a guest run executes Bitcoin Core's own code.

Reads risc0 execute-mode profiles (RISC0_PPROF_OUT: gzipped pprof protobuf) with a hand-written
decoder (no protobuf package needed) and attributes every cycle to its LEAF function (self time),
then to an owner by symbol:

  core      Bitcoin Core C++ (script interpreter, sighash, serialisation, CTransaction, hashing, pow,
            merkle, chainparams): every C++ symbol not ours
  libsecp   libsecp256k1 C (secp256k1_*), including the GHOST coprocessor backends that live in it
  libzync   code written for libzync: the batch MSM, ECMH, agg-lite, wire decode (method::batch_ecdsa,
            ::swiftsync, ::agg_lite, ::lz_wire, ::ss_hints, ::msm) and the guest C++ helpers in
            verify_input.cpp that are ours (hz_*, cached_tx, verify_inputs_batch_once, tx_out_leaves ...)
  hazync    the rest of the guest's own Rust (method::*) and the coinbase SMT / utreexo
  risc0     zkVM runtime, syscalls, serde, Rust std/alloc/core, memcpy/memset, paging ("PageIn" etc.)

Prints a table per profile and the totals. Classification is by name, so it is approximate at the
edges (inlined code is charged to the function it was inlined into); the leaf set is printed with -v.
"""
import gzip, sys
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
            n, i = varint(b, i)
            v = b[i:i + n]; i += n
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
    try:
        raw = gzip.decompress(raw)
    except OSError:
        pass
    strings, samples, locs, funcs = [], [], {}, {}
    for f, t, v in fields(raw):
        if f == 6:
            strings.append(v.decode("utf-8", "replace"))
        elif f == 2:
            ids, vals = [], []
            for ff, tt, vv in fields(v):
                if ff == 1: ids += packed(vv, tt)
                elif ff == 2: vals += packed(vv, tt)
            samples.append((ids, vals))
        elif f == 4:
            lid, fn = None, []
            for ff, tt, vv in fields(v):
                if ff == 1: lid = vv
                elif ff == 4:
                    for g, _, w in fields(vv):
                        if g == 1: fn.append(w)
            locs[lid] = fn
        elif f == 5:
            fid = name = None
            for ff, tt, vv in fields(v):
                if ff == 1: fid = vv
                elif ff == 2: name = vv
            funcs[fid] = name
    self_ = Counter()
    for ids, vals in samples:
        if not ids or not vals:
            continue
        fn = locs.get(ids[0]) or [None]
        name = strings[funcs.get(fn[0], 0)] if fn[0] is not None else "?"
        self_[name] += vals[0]
    return self_


OURS_CPP = ("hz_", "cached_tx", "verify_inputs_batch", "tx_out_leaves", "tx_wtxid_info", "tx_full_sigops",
            "tx_wu_sigops", "tx_vin_count", "tx_input_prevout_txid", "check_tx", "is_final_tx", "coin_leaf",
            "MiniReader", "hazync_", "check_witness_commitment", "check_bip34", "check_pow", "merkle_root",
            "calc_next_bits", "add_work", "block_subsidy", "coinbase_value", "is_coinbase_tx", "core_")
LIBZYNC_RS = ("batch_ecdsa", "swiftsync", "agg_lite", "lz_wire", "ss_hints", "msm", "liftx_hint")
RISC0 = ("risc0", "serde", "core::", "alloc::", "std::", "memcpy", "memset", "memmove", "PageIn", "PageOut",
         "sys_", "__", "compiler_builtins", "<core", "<alloc", "<std", "hashbrown", "bytemuck")


def owner(name):
    n = name or "?"
    if n.startswith("secp256k1_") or n.startswith("rustsecp"):
        return "libsecp"
    if any(n.startswith(p) or f"::{p}" in n for p in ("method::" + x for x in LIBZYNC_RS)) or any(
            f"method::{x}::" in n for x in LIBZYNC_RS):
        return "libzync"
    if n.startswith(OURS_CPP):
        return "libzync" if n.startswith(("hz_", "cached_tx", "verify_inputs_batch", "hazync_")) else "hazync"
    if n.startswith("method::") or "utreexo" in n or "smt" in n or "bip30" in n:
        return "hazync"
    if any(n.startswith(p) or p in n[:40] for p in RISC0):
        return "risc0"
    return "core"  # remaining C++ (Core's classes and free functions)


def main():
    verbose = "-v" in sys.argv
    tot = Counter()
    for p in [a for a in sys.argv[1:] if a != "-v"]:
        s = load(p)
        by = Counter()
        for n, c in s.items():
            by[owner(n)] += c
        total = sum(by.values())
        tot.update(by)
        print(f"{p}: {total:,} cycles")
        for k, c in by.most_common():
            print(f"  {k:8s} {c:>15,}  {100 * c / total:5.1f}%")
        if verbose:
            for n, c in s.most_common(60):
                print(f"    {owner(n):8s} {c:>13,}  {n[:110]}")
    if len(sys.argv) > 2:
        total = sum(tot.values())
        print(f"ALL: {total:,} cycles")
        for k, c in tot.most_common():
            print(f"  {k:8s} {c:>15,}  {100 * c / total:5.1f}%")


if __name__ == "__main__":
    main()
