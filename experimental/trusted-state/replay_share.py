#!/usr/bin/env python3
"""replay_share.py <profile.pprof>...  -- TRUSTED_STATE.md §11.3 (measured basis for an ESTIMATE)

How much of a guest run is script and signature work, which a "replay pass" (re-deriving the main lineage's coin log,
trie and BIP30 tree over blocks the anchored lineage already proved valid) would not repeat. Uses core_share.py's own
pprof decoder (experimental/libzync-bench/cost). Unlike core_share.py, which charges a sample to its innermost
non-runtime frame, this charges it by its WHOLE stack: a sample is SKIPPABLE if any frame on its stack is script or
signature verification (Core's interpreter, sighash, the signature checkers, libsecp, the field and MSM shims, the
batch-ECDSA module, the per-input verify wrappers), else KEPT (transaction parsing, txid/wtxid, CheckTransaction,
coin leaves, journal, coin log, ECMH, trie, BIP30, block rules, paging and memcpy under them).
"""
import os, sys
from collections import Counter

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "libzync-bench", "cost"))
from core_share import fields, packed  # noqa: E402
import gzip  # noqa: E402

SKIP = ("EvalScript", "VerifyScript", "VerifyWitnessProgram", "ExecuteWitnessScript", "EvalChecksig", "SignatureHash",
        "SignatureChecker", "CheckSig", "CheckECDSA", "CheckSchnorr", "CheckMultiSig", "PrecomputedTransactionData",
        "secp256k1_", "hazync_fq", "hazync_ecmult", "hazync_batch", "hazync_lift", "hazync_scalar", "batch_ecdsa",
        "::msm", "liftx", "CPubKey", "XOnlyPubKey", "ripemd160", "CRIPEMD160", "CHash160", "CSHA1")


def stacks(path):
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
    for ids, vals in samples:
        if ids and vals:
            yield [strings[funcs.get(fid, 0)] for lid in ids for fid in (locs.get(lid) or [])], vals[0]


def bare_method(names):
    """The innermost non-runtime frame is the guest crate's bare `method[..]` frame: code inlined into the crate root,
    which core_share.py charges to the batch-signature finish in a chunk (its total matches the finish the guest logs)."""
    from core_share import owner
    for n in names:
        if owner(n) != "runtime":
            return n.startswith("method[") and n.endswith("]") and "::" not in n
    return False


def main():
    for p in sys.argv[1:]:
        c, top = Counter(), Counter()
        for names, v in stacks(p):
            hit = next((m for m in SKIP for n in names if m in n), None)
            if not hit and "chunk" in os.path.basename(p) and bare_method(names):
                hit = "bare method[] (batch finish, inlined)"
            c["skip" if hit else "keep"] += v
            if hit: top[hit] += v
        t = sum(c.values())
        bare = top["bare method[] (batch finish, inlined)"]
        print(f"{os.path.basename(p)}: {t:,} cycles  skippable {c['skip']:,} ({100 * c['skip'] / t:.1f}%; without the bare "
              f"frame {100 * (c['skip'] - bare) / t:.1f}%)  kept {c['keep']:,} ({100 * c['keep'] / t:.1f}%)")
        print("    by first marker: " + ", ".join(f"{k} {100 * v / t:.1f}%" for k, v in top.most_common(8)))


if __name__ == "__main__":
    main()
