"""prof_split.py <profile.pb> [...] [--leaves N] — where a guest's cycles go, by category, with paging apart.

Reads risc0 execute-mode profiles (RISC0_PPROF_OUT: gzipped pprof protobuf) with the hand-written decoder
of ../cost/core_share.py (no protobuf package needed) and prints, per profile:

  1. the total, and the paging share: samples whose LEAF frame is [PageIn] or [PageOut];
  2. a category table over the NON-paging cycles. Each sample is charged to the innermost frame that is
     not runtime (core_share.owner), so memcpy, malloc and the SHA accelerator's syscall count for the
     code that called them. The category is then decided by that frame's (mangled) name, see CATS;
  3. the same table with paging charged to callers too (core_share.load's own convention: the method the
     28.10 private profile's 75.4% / 0.3% figures were computed with), so both readings are on record;
  4. the top SELF (leaf-frame) functions, paging included, uncharged.

Classification is by symbol name, so it is approximate at the edges: code inlined into a function is
charged to that function. `--leaves N` lists the N largest charged frames of every category so the
keyword lists can be audited; `--sum LABEL` prints ONE table over all the profiles given (a block's chunk,
trie update and aggregate together).
"""
import gzip, os, subprocess, sys
from collections import Counter

sys.dont_write_bytecode = True  # no __pycache__ beside core_share.py
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "cost"))
import core_share as cs  # noqa: E402  (varint/fields/packed decoder and owner())

PAGING = ("[PageIn]", "[PageOut]")

# Order matters: the first category whose keyword occurs in the charged frame's name wins.
# Script first, so Core's VerifyScript / VerifyWitnessProgram are not read as signature work.
CATS = [
    ("script interpreter", ("EvalScript", "VerifyScript", "VerifyWitnessProgram", "ExecuteWitnessScript",
                            "EvalChecksig", "EvalTapscript", "GetScriptOp", "CScriptNum", "CastToBool",
                            "FindAndDelete", "CheckSignatureEncoding", "IsValidSignatureEncoding",
                            "CheckPubKeyEncoding", "IsLowDERSignature", "IsDefinedHashtypeSignature",
                            "CheckMinimalPush", "CheckLockTime", "CheckSequence", "CScript::",
                            "ConditionStack", "popstack", "CountWitnessSigOps", "WitnessSigOps", "IsWitnessProgram",
                            "IsPayToScriptHash", "IsPushOnly", "set_success", "set_error")),
    ("signatures", ("secp256k1", "hazync_fq", "hazync_ecmult", "hazync_liftx", "hazync_scalar", "batch_ecdsa",
                    "hazync_batch", "msm", "Msm", "lift_x", "liftx", "CPubKey", "XOnlyPubKey", "SignatureChecker",
                    "VerifyECDSASignature", "VerifySchnorrSignature", "CheckECDSASignature",
                    "CheckSchnorrSignature", "ecdsa", "schnorr", "Ecdsa", "Schnorr", "bigint2",
                    # libsecp's field backend helpers (prover/methods/guest/field_bigint2_impl.h, both releases)
                    "hz_add", "hz_canon", "hz_fold", "hz_ge_p", "hz_is_zero_mod", "hz_neg", "hz_sub_p")),
    ("hashing", ("SHA256", "SHA1", "SHA512", "sha2", "sha256", "Sha256", "RIPEMD", "ripemd", "Hash", "hash",
                 "merkle", "Merkle", "sys_sha", "TaggedHash", "Finalize", "SHA256D64", "hz_d64")),
    ("tx parse/serialise", ("Unserialize", "Serialize", "CTransaction", "CMutableTransaction", "CTxIn", "CTxOut",
                            "COutPoint", "CScriptWitness", "cached_tx", "Formatter", "Reader", "SpanReader",
                            "DataStream", "VectorWriter", "MiniReader", "ReadCompactSize", "WriteCompactSize",
                            "prevector",
                            "serde", "wire", "deserial", "decode", "Decode", "PrecomputedTransactionData")),
    ("state (coins/trie/ECMH/SMT)", ("agg_lite", "swiftsync", "ecmh", "Ecmh", "trie", "Trie", "coinlog",
                                     "utreexo", "Stump", "stump", "smt", "Smt", "coin_leaf", "tx_out_leaves", "ttree",
                                     "hz_leaf")),
]
LIBSECP = ("secp256k1_", "hazync_fq", "hazync_ecmult", "hazync_lift", "hazync_scalar", "hz_add", "hz_canon", "hz_fold",
           "hz_ge_p", "hz_is_zero_mod", "hz_neg", "hz_sub_p")
# A bare `method[<hash>]` frame is Rust code inlined into the guest crate's root (its main): reading the input,
# committing the journal (v1: risc0 serde), resolving assumptions (aggregate). No name says more.
CRATE_ROOT = "guest main / I/O (crate root)"


def is_sig(n):
    """Signature verification, for the inclusive measure: libsecp + its field backend, v2's batch ECDSA (Rust) + hook."""
    return n.startswith(LIBSECP) or "hazync_batch" in n or "::batch_ecdsa" in n


def demangle_all(names):
    """{mangled: demangled} through binutils' c++filt (one process); identity where it is missing."""
    names = sorted(n for n in names if n.startswith("_Z"))
    try:
        out = subprocess.run(["c++filt"], input="\n".join(names), capture_output=True, text=True, check=True).stdout
        dem = out.split("\n")
        if len(dem) >= len(names):
            return dict(zip(names, dem))
    except (OSError, subprocess.CalledProcessError):
        print("   ⚠ c++filt not found: C++ frames are matched on their MANGLED names (argument types included)")
    return {}


def fname(demangled):
    """`uint256 SignatureHash<CTransaction>(CScript const&, ...)` -> `SignatureHash<CTransaction>`: the function's
    own qualified name, so keywords never match argument or return types."""
    depth, end = 0, len(demangled)
    for i, ch in enumerate(demangled):
        if ch == "<": depth += 1
        elif ch == ">": depth -= 1
        elif ch == "(" and depth == 0:
            end = i; break
    head, depth, cut = demangled[:end], 0, 0
    for i, ch in enumerate(head):
        if ch == "<": depth += 1
        elif ch == ">": depth -= 1
        elif ch == " " and depth == 0: cut = i + 1
    return head[cut:] or demangled


def category(name):
    if name.startswith("method[") and name.endswith("]") and "::" not in name:
        return CRATE_ROOT
    for c, ks in CATS:
        if any(k in name for k in ks):
            return c
    return "other"


def load_stacks(path):
    """[(cycles, [frame names, innermost first])] from a risc0 pprof."""
    raw = open(path, "rb").read()
    try:
        raw = gzip.decompress(raw)
    except OSError:
        pass
    strings, samples, locs, funcs = [], [], {}, {}
    for f, t, v in cs.fields(raw):
        if f == 6:
            strings.append(v.decode("utf-8", "replace"))
        elif f == 2:
            ids, vals = [], []
            for ff, tt, vv in cs.fields(v):
                if ff == 1: ids += cs.packed(vv, tt)
                elif ff == 2: vals += cs.packed(vv, tt)
            samples.append((ids, vals))
        elif f == 4:
            lid, fn = None, []
            for ff, tt, vv in cs.fields(v):
                if ff == 1: lid = vv
                elif ff == 4:
                    for g, _, w in cs.fields(vv):
                        if g == 1: fn.append(w)
            locs[lid] = fn
        elif f == 5:
            fid = name = None
            for ff, tt, vv in cs.fields(v):
                if ff == 1: fid = vv
                elif ff == 2: name = vv
            funcs[fid] = name
    out = []
    for ids, vals in samples:
        if not ids or not vals:
            continue
        frames = [strings[funcs.get(fid, 0)] for lid in ids for fid in (locs.get(lid) or [])]
        out.append((vals[0], frames or ["?"]))
    return out


def charged(frames):
    """core_share.load's rule: the innermost frame whose owner is not 'runtime', else the leaf."""
    for n in frames:
        if cs.owner(n) != "runtime":
            return n
    return frames[0]


def top(counter, n):
    """The n largest, ties broken by name, so the listing does not depend on the pprof's sample order."""
    return sorted(counter.items(), key=lambda x: (-x[1], x[0]))[:n]


def report(label, st, nleaves):
    total = sum(c for c, _ in st)
    paging = sum(c for c, fr in st if fr[0] in PAGING)
    body = total - paging
    dem = demangle_all({n for _, fr in st for n in fr})
    cat_np, cat_all, leaf, per = Counter(), Counter(), Counter(), {}
    for c, fr in st:
        raw = charged(fr)
        name = fname(dem[raw]) if raw in dem else raw
        k = category(name)
        cat_all[k] += c
        leaf[fr[0]] += c
        if fr[0] not in PAGING:
            cat_np[k] += c
            per.setdefault(k, Counter())[name] += c
    print(f"== {label}")
    print(f"   total (profile, incl. paging) {total:>15,}")
    print(f"   paging [PageIn]+[PageOut]     {paging:>15,}  {100 * paging / total:5.1f}% of total")
    print(f"   excl. paging                  {body:>15,}")
    order = [c for c, _ in CATS] + [CRATE_ROOT, "other"]
    print(f"   {'category':30s} {'cycles excl. paging':>20s} {'% excl.':>8s} | {'paging charged to callers':>26s} {'%':>6s}")
    for k in order:
        print(f"   {k:30s} {cat_np[k]:>20,} {100 * cat_np[k] / body:7.1f}% | {cat_all[k]:>26,} {100 * cat_all[k] / total:5.1f}%")
    # Inclusive ("cumulative") shares: every sample with such a frame ANYWHERE on its stack.
    lib = sum(c for c, fr in st if fr[0] not in PAGING and any(n.startswith(LIBSECP) for n in fr))
    libp = sum(c for c, fr in st if any(n.startswith(LIBSECP) for n in fr))
    ev = sum(c for c, fr in st if "EvalScript" in fr[0])
    sig = sum(c for c, fr in st if fr[0] not in PAGING and any(is_sig(n) for n in fr))
    print("   inclusive: libsecp256k1 and its field backend on the stack (secp256k1_* / hazync_* / field hz_*):")
    print(f"     excl. paging {lib:>15,} = {100 * lib / body:5.1f}% of non-paging, {100 * lib / total:5.1f}% of total;"
          f"  incl. paging {libp:>15,} = {100 * libp / total:5.1f}% of total")
    print(f"   inclusive: libsecp or batch ECDSA on the stack, excl. paging {sig:>15,} = {100 * sig / body:5.1f}% of non-paging,"
          f" {100 * sig / total:5.1f}% of total")
    print(f"   EvalScript SELF (leaf) {ev:>12,} = {100 * ev / body:5.2f}% of non-paging, {100 * ev / total:5.2f}% of total")
    print("   top self (leaf) functions, paging included:")
    for n, c in top(leaf, 15):
        print(f"     {c:>14,} {100 * c / total:5.1f}%  {(fname(dem[n]) if n in dem else n)[:120]}")
    if nleaves:
        for k in order:
            print(f"   -- {k}: largest charged frames")
            for n, c in top(per.get(k, Counter()), nleaves):
                print(f"     {c:>14,} {100 * c / body:5.1f}%  {n[:120]}")
    print()


def main():
    a = sys.argv[1:]
    n = 0
    if "--leaves" in a:
        i = a.index("--leaves"); n = int(a[i + 1]); del a[i:i + 2]
    if "--sum" in a:  # one table over several profiles (e.g. a block's chunk + trie update + aggregate)
        i = a.index("--sum"); label = a[i + 1]; del a[i:i + 2]
        report(label + " = " + " + ".join(os.path.basename(p) for p in a), [x for p in a for x in load_stacks(p)], n)
        return
    for p in a:
        report(os.path.basename(p), load_stacks(p), n)


if __name__ == "__main__":
    main()
