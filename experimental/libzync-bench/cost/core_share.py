"""core_share.py <profile.pb> [...] — how much of a guest run executes Bitcoin Core's own code.

Reads risc0 execute-mode profiles (RISC0_PPROF_OUT: gzipped pprof protobuf) with a hand-written
decoder (no protobuf package needed) and attributes every cycle to the INNERMOST NON-RUNTIME frame of
its stack (so memcpy, paging and the SHA accelerator count for the code that called them), then to an
owner by symbol:

  core      Bitcoin Core C++ (interpreter, sighash, serialisation, CTransaction, CSHA256, pow, merkle,
            chainparams): every C++ symbol that is not our wrapper
  libsecp   libsecp256k1 C (secp256k1_*)
  accel     the coprocessor shims libsecp calls (hazync_fq_*, lift_x hint, scalar inverse): GHOST levers
  libzync   written for libzync: batch MSM/finish, ECMH, agg-lite, wire decode (Rust), and the C++
            helpers cached_tx / hz_* / verify_inputs_batch_once / tx_out_leaves
  hazync    the rest of the guest crate (Rust) and verify_input.cpp's other wrappers
  runtime   paging, syscalls (sys_sha_compress = the SHA accelerator), memcpy/memset/memcmp, malloc,
            serde, Rust core/alloc. SELF time: memcpy called by Core is charged here, not to Core.

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
        # Walk from the leaf outwards and charge the sample to the first frame that is not runtime, so
        # memcpy, paging and the SHA accelerator are charged to the code that asked for them. Frames of
        # one location (inlining) are innermost first.
        name = "?"
        for lid in ids:
            for fid in (locs.get(lid) or []):
                cand = strings[funcs.get(fid, 0)]
                if owner(cand) != "runtime":
                    name = cand
                    break
            else:
                continue
            break
        if name == "?":
            fn = locs.get(ids[0]) or [None]
            name = strings[funcs.get(fn[0], 0)] if fn[0] is not None else "?"
        self_[name] += vals[0]
    return self_


# Our C++ wrapper (prover/methods/guest/verify_input.cpp). Mangled C++ names contain the plain identifier.
LIBZYNC_CPP = ("cached_tx", "cached_spent", "hz_", "verify_inputs_batch_once", "tx_out_leaves", "hazync_batch")
HAZYNC_CPP = ("verify_input", "verify_inputs_batch", "check_tx", "is_final_tx", "tx_full_sigops", "tx_wu_sigops",
              "tx_vin_count", "tx_input_prevout_txid", "tx_wtxid_info", "tx_seqs_batch", "check_witness_commitment",
              "check_bip34", "check_pow", "merkle_root", "calc_next_bits", "add_work", "block_subsidy",
              "coinbase_value", "is_coinbase_tx", "coin_leaf")
# NOT "MiniReader": Core's Unserialize templates instantiated with our stream are Core's code.
LIBZYNC_RS = ("batch_ecdsa", "swiftsync", "agg_lite", "lz_wire", "ss_hints", "msm")
RUNTIME = ("[PageIn]", "[PageOut]", "memcpy", "memset", "memmove", "memcmp", "malloc", "free", "calloc", "realloc",
           "_Znwj", "_Znaj", "_ZdlPv", "_ZdaPv", "sys_", "__", "risc0", "serde", "core::", "alloc::", "std::",
           "<core", "<alloc", "<std", "compiler_builtins", "hashbrown", "bytemuck", "abort", "strlen")


def owner(name):
    """core | libsecp | accel | libzync | hazync | runtime. See the module docstring."""
    n = name or "?"
    if n.startswith(RUNTIME) or n.startswith("<risc0") or n in ("?",):
        return "runtime"
    if n.startswith("secp256k1_"):
        return "libsecp"
    # Coprocessor shims libsecp calls (GHOST: field backend, lift_x hint, scalar inverse): Rust, used BY libsecp.
    if n.startswith("hazync_") and not n.startswith("hazync_batch"):
        return "accel"
    if n.startswith("_Z"):  # C++
        if any(k in n for k in LIBZYNC_CPP):
            return "libzync"
        if any(k in n for k in HAZYNC_CPP):
            return "hazync"
        return "core"
    if n.startswith(LIBZYNC_CPP) or n.startswith("hazync_batch"):
        return "libzync"
    if n.startswith(HAZYNC_CPP):
        return "hazync"
    if "method[" in n:  # the guest crate (Rust)
        # A bare `method[..]` frame is code inlined into the crate root. In a libzync chunk its total
        # matches the batch finish the guest itself logs (chunk 1: 13.9 M + Acc 5.7 M vs finish 20.4 M),
        # so it is charged to libzync.
        if n.rstrip("]").endswith(n.split("[")[1].split("]")[0]) or n.endswith("]"):
            return "libzync"
        return "libzync" if any(f"::{m}" in n for m in LIBZYNC_RS) else "hazync"
    if n.startswith("sha2[") or "risc0_crypto" in n or "risc0_bigint2" in n:
        return "hazync"
    return "runtime"


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
