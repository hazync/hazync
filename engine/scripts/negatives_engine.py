#!/usr/bin/env python3
"""negatives_engine.py <product host> <hzengine> <keys> <metering keys> <block_962000.json> <out dir>: libzync CI's
negative tests on block 962,000 (scripts/libzync/ci/negatives.py, the same edits) on the ENGINE (roadmap 1.5).

Each case's block goes through `engine-export` (the exact guest inputs, with RISC Zero's journal for each). Chunk cases
(HAZYNC_CHUNKS=1, the honest block's signature hints reused, as negatives.py): the engine's chunk journal must be
byte-identical to RISC Zero's, AND RISC Zero's verdict (all_valid, from `exec-chunk`) the wanted one. Aggregate cases:
the block's collect run (agg_collect.in, where RISC Zero refuses it) must be refused by the engine (metering image:
env::verify accepts without proofs) with RISC Zero's own refusal line; the honest block's collect run is the control. The honest block first, valid at both levels.
Prints NEGATIVES_ENGINE_OK only if every case is as expected."""
import json, os, re, struct, subprocess, sys
H, E, K, KB, B, O = (os.path.abspath(a) for a in sys.argv[1:7])
os.makedirs(O, exist_ok=True)
base = json.load(open(B))
env = {k: v for k, v in os.environ.items() if not k.startswith("HAZYNC_")}
env.update(HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS="1", HAZYNC_HINT_THREADS=os.environ.get("HAZYNC_HINT_THREADS", "8"))
# the coin log's chain transaction counts, as blocks.sh exports them for negatives.py
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "../.."))
env["HAZYNC_COINLOG_TXCOUNTS"] = os.environ.get("HAZYNC_COINLOG_TXCOUNTS", os.path.join(REPO, "experimental/libzync-bench/cost/txcounts.txt"))
env.pop("RISC0_DEV_MODE", None)
bad = 0

def run(cmd, e, log):
    r = subprocess.run(cmd, env=e, capture_output=True, text=True, errors="replace")
    open(log, "w").write(r.stdout + r.stderr)
    return r.returncode, r.stdout + r.stderr

def verdict(ok, name, detail):
    global bad
    print(("PASS" if ok else "WRONG"), name, detail, flush=True)
    bad += 0 if ok else 1

def export(f, name, lz):
    x = f"{O}/x_{name}"
    rc, out = run([H, "engine-export", x], dict(env, HAZYNC_BLOCK=f, HAZYNC_LZ_HINTS=lz), f"{O}/{name}.export.log")
    return x, rc, out

def engine_exec(keys, inp, journal, self_id=False):
    # the guest's own stdout (its refusal line among it) goes to --log, not to hzengine's output: both are returned
    log = journal + ".guest.log"
    cmd = [E, "execute", "--dir", keys, "--input", inp, "--journal", journal, "--log", log] + (["--self-id"] if self_id else [])
    r = subprocess.run(cmd, capture_output=True, text=True, errors="replace")
    g = open(log, errors="replace").read() if os.path.exists(log) else ""
    return r.returncode, re.sub(r"\x1b\[[0-9;]*m", "", r.stdout + r.stderr + "\n" + g)

AGG_REFUSAL = "aggregate: bv=false ph=true carry=true rt=true time=true smt=true"

def refusal(text):
    return next((l.strip() for l in text.splitlines() if "aggregate: bv=" in l and "panicked" not in l), None)

# the honest block: hints, then valid at both levels
rc, _ = run([H, "libzync-hints", f"{O}/base.lzb"], dict(env, HAZYNC_BLOCK=B), f"{O}/base.hints.log")
verdict(rc == 0, "base hints", f"rc {rc}")
x, rc, _ = export(B, "base", f"{O}/base.lzb")
erc, eo = engine_exec(K, f"{x}/chunk_00.in", f"{x}/chunk_00.j")
same = erc == 0 and open(f"{x}/chunk_00.j", "rb").read() == open(f"{x}/chunk_00.r0journal", "rb").read()
verdict(same, "base chunk (engine == RISC Zero)", f"rc {erc}")
arc, ao = engine_exec(KB, f"{x}/agg.in", f"{x}/agg.j", True)
verdict(arc == 0 and "INVALID" not in ao, "base block valid (engine aggregate)", f"rc {arc}")
# control for the aggregate cases: the honest block's collect stream is NOT refused by the engine
crc, co = engine_exec(KB, f"{x}/agg_collect.in", f"{x}/agg_collect.j", True)
# a finished collect run ends with exit code COLLECT_EXIT (libzync::wire, 2); the platform reports it as "guest exit(2)"
ok = os.path.exists(f"{x}/agg_collect.in") and refusal(co) is None and "guest exit(2)" in co
verdict(ok, "base collect run finishes, not refused (engine)", f"rc {crc}: {'guest exit(2) = COLLECT_EXIT' if 'guest exit(2)' in co else (co.strip().splitlines() or [''])[-1][:120]}")

def chunk(name, ti, inp, edit, want):
    b = json.loads(json.dumps(base)); edit(b["txs"][ti]["prevouts"][inp])
    f = f"{O}/neg_{name}.json"; json.dump(b, open(f, "w"))
    rc, r0 = run([H, "exec-chunk", "0"], dict(env, HAZYNC_BLOCK=f, HAZYNC_LZ_HINTS=f"{O}/base.lzb"), f"{O}/{name}.r0.log")
    m = re.search(r"all_valid=(\d)", r0); r0v = m.group(1) if m else f"none (rc {rc})"
    x, xrc, _ = export(f, name, f"{O}/base.lzb")
    erc, eo = engine_exec(K, f"{x}/chunk_00.in", f"{x}/chunk_00.j")
    same = erc == 0 and os.path.exists(f"{x}/chunk_00.r0journal") and \
        open(f"{x}/chunk_00.j", "rb").read() == open(f"{x}/chunk_00.r0journal", "rb").read()
    verdict(same and r0v == want, name, f"RISC Zero all_valid={r0v} (want {want}); engine journal {'IDENTICAL' if same else 'DIFFERENT / failed (rc %d)' % erc}")
    os.remove(f)

setp = lambda **k: (lambda p: p.update(k))
v = base["txs"][4]["prevouts"][0]["value"]
chunk("maturity_ok", 4, 0, setp(coin_is_coinbase=1, coin_height=961000), "1")
chunk("maturity_immature", 4, 0, setp(coin_is_coinbase=1, coin_height=961950), "0")
chunk("height_lock_met", 304, 0, setp(coin_height=961999), "1")
chunk("height_lock_unmet", 304, 0, setp(coin_height=962000), "0")
chunk("value_more", 4, 0, setp(value=v + 1000), "1")
chunk("value_below_outputs", 4, 0, setp(value=0), "0")

def agg(name, edit, flag):
    # The block's coin-log collect run (the aggregate without ECMH hints) is where RISC Zero refuses it, before any
    # commit: the export writes that run's stream (agg_collect.in) first. The engine must refuse the same stream with
    # the same line. The published image's aggregate logs only its summary, so the refusal is the block-validity bit
    # alone, every other check passing; which rule (here `flag`) is the rig's to show against Core.
    b = json.loads(json.dumps(base)); edit(b); f = f"{O}/agg_{name}.json"; json.dump(b, open(f, "w"))
    run([H, "libzync-hints", f"{O}/{name}.lzb"], dict(env, HAZYNC_BLOCK=f), f"{O}/{name}.hints.log")
    x, xrc, xo = export(f, name, f"{O}/{name}.lzb")
    r0 = refusal(xo)
    if not os.path.exists(f"{x}/agg_collect.in"):
        verdict(False, name, "no collect stream exported"); os.remove(f); return
    arc, ao = engine_exec(KB, f"{x}/agg_collect.in", f"{x}/agg_collect.j", True)
    eng = refusal(ao)
    ok = xrc != 0 and r0 == AGG_REFUSAL and arc != 0 and eng == AGG_REFUSAL
    verdict(ok, name, f"({flag}) RISC Zero: {r0 or 'no refusal (rc %d)' % xrc} | engine rc {arc}: {eng or 'no refusal'}")
    os.remove(f)

def rich(b):
    x = bytearray.fromhex(b["coinbase_hex"]); o = 6; o += 1 + 36; L = x[o]; o += 1 + L + 4; o += 1
    val = struct.unpack_from("<Q", x, o)[0]; struct.pack_into("<Q", x, o, val + 5_000_000_000); b["coinbase_hex"] = x.hex()

agg("bad_nonce_pow", lambda b: b.update(nonce=b["nonce"] ^ 1), "pow")
agg("coinbase_overpays", rich, "subsidy")
print("NEGATIVES_ENGINE_OK" if bad == 0 else f"NEGATIVES_ENGINE_FAILED ({bad})")
sys.exit(1 if bad else 0)
