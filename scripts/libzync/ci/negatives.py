#!/usr/bin/env python3
"""libzync CI: negative tests on a real block (962,000), each paired with a control that must pass.

Chunk level (exec-chunk, the whole block as one chunk; the honest block's signature hints reused, since none of these
edits changes a signature): coin maturity (a coinbase coin one block short), a relative height lock one block short,
an input value below the outputs; each beside the same edit made valid. Aggregate level (agg-exec-fake, hints rebuilt):
a header nonce that breaks proof of work, a coinbase paying 50 BTC too much, each refused at the aggregate's
block-validity check (which rule is the rig's to show, against Core). The honest block must be valid first.
Ported from experimental/libzync-bench/core_rules/neg_962000.py and neg_aggregate_962000.py (11-01).

usage: negatives.py <mainnet host> <block_962000.json> <out dir>; prints NEGATIVES_OK only if every case is as expected
"""
import json, os, re, struct, subprocess, sys
H, B, O = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2]), os.path.abspath(sys.argv[3])
os.makedirs(O, exist_ok=True)
base = json.load(open(B))
env = dict(os.environ, HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS="1", HAZYNC_HINT_THREADS=os.environ.get("HAZYNC_HINT_THREADS", "2"))
env.pop("RISC0_DEV_MODE", None)
bad = 0
def run(args, e, log):
    r = subprocess.run([H] + args, env=e, capture_output=True, text=True, errors="replace")
    open(log, "w").write(r.stdout + r.stderr)
    return r.returncode, r.stdout + r.stderr
def verdict(ok, name, detail):
    global bad
    print(("PASS" if ok else "WRONG"), name, detail, flush=True)
    bad += 0 if ok else 1
def hints(f, name):
    rc, x = run(["libzync-hints", f"{O}/{name}.lzb"], dict(env, HAZYNC_BLOCK=f), f"{O}/{name}.hints.log")
    return rc
# the honest block: hints, then valid at both levels
rc = hints(B, "base")
verdict(rc == 0, "base hints", f"rc {rc}")
rc, x = run(["agg-exec-fake"], dict(env, HAZYNC_BLOCK=B, HAZYNC_LZ_HINTS=f"{O}/base.lzb"), f"{O}/base.agg.log")
verdict("block validation cycles" in x and "INVALID" not in x and "panicked" not in x, "base block valid (aggregate)", f"rc {rc}")
# chunk level
def chunk(name, ti, inp, edit, want):
    b = json.loads(json.dumps(base)); edit(b["txs"][ti]["prevouts"][inp])
    f = f"{O}/neg_{name}.json"; json.dump(b, open(f, "w"))
    rc, x = run(["exec-chunk", "0"], dict(env, HAZYNC_BLOCK=f, HAZYNC_LZ_HINTS=f"{O}/base.lzb"), f"{O}/{name}.log")
    m = re.search(r"all_valid=(\d)", x); got = m.group(1) if m else f"none (rc {rc})"
    verdict(got == want, name, f"all_valid={got} (want {want})"); os.remove(f)
setp = lambda **k: (lambda p: p.update(k))
v = base["txs"][4]["prevouts"][0]["value"]
chunk("maturity_ok", 4, 0, setp(coin_is_coinbase=1, coin_height=961000), "1")
chunk("maturity_immature", 4, 0, setp(coin_is_coinbase=1, coin_height=961950), "0")
chunk("height_lock_met", 304, 0, setp(coin_height=961999), "1")
chunk("height_lock_unmet", 304, 0, setp(coin_height=962000), "0")
chunk("value_more", 4, 0, setp(value=v + 1000), "1")
chunk("value_below_outputs", 4, 0, setp(value=0), "0")
# aggregate level
def agg(name, edit, flag):
    b = json.loads(json.dumps(base)); edit(b); f = f"{O}/agg_{name}.json"; json.dump(b, open(f, "w"))
    hints(f, name)
    rc, x = run(["agg-exec-fake"], dict(env, HAZYNC_BLOCK=f, HAZYNC_LZ_HINTS=f"{O}/{name}.lzb"), f"{O}/{name}.agg.log")
    # An agg-lite image logs each rule (block INVALID ... pow= subsidy= ...); the published image's aggregate logs only
    # its summary, so there the refusal must be the block-validity bit alone, every other aggregate check passing.
    # Which rule a block breaks is the rig's to show, rule by rule against Core (high-hash, bad-cb-amount, ...).
    m = re.search(r"block INVALID (all_ok=\w+ pow=\w+ merkle=\w+ subsidy=\w+.*?bip30=\w+)", x, re.S)
    if m:
        fl = " ".join(m.group(1).split()); ok = f"{flag}=false" in fl
    else:
        fl = next((l.strip() for l in x.splitlines() if "Guest panicked: aggregate:" in l), None)
        ok = bool(fl) and fl.endswith("aggregate: bv=false ph=true carry=true rt=true time=true smt=true")
    verdict(ok and rc != 0, name, f"| {fl or 'no refusal (rc %d)' % rc}"); os.remove(f)
def rich(b):
    x = bytearray.fromhex(b["coinbase_hex"]); o = 6; o += 1 + 36; L = x[o]; o += 1 + L + 4; o += 1
    val = struct.unpack_from("<Q", x, o)[0]; struct.pack_into("<Q", x, o, val + 5_000_000_000); b["coinbase_hex"] = x.hex()
agg("bad_nonce_pow", lambda b: b.update(nonce=b["nonce"] ^ 1), "pow")
agg("coinbase_overpays", rich, "subsidy")
print("NEGATIVES_OK" if bad == 0 else f"NEGATIVES_FAILED ({bad})")
sys.exit(1 if bad else 0)
