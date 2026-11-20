#!/usr/bin/env python3
"""D1, the tip run (docs/libzync/REORG.md §3, §14 decision 1, §19): the board follows TWO prep runs. The board pass serves
blocks 1..X-1, the tip run every block from X = HAZYNC_LZ_TIP_FROM on; each has its own states, bundles, reorg records
and ack directory. A fake `host verify-any` checks a receipt against the states in HAZYNC_LZ_STATE_DIR, as lz_coord.rs
does (the one in test_reorg.py).

X = 20. The board pass has prepped 1..12 (it is paced by proving: far behind); the tip run has prepped 1..30 from
genesis, with bundles for 20..30 and one sample bundle below X (leaf_5.bin, which must not be served). Checks:
  - configuration: one ack directory for both runs, the same state directory, a missing directory: refused (exit 3 at
    start-up, a real server process)
  - bundles: block 5 is the board pass's, 25 the tip run's, 15 (the gap) nobody's; the servable high is 30, the board
    pass's 12
  - claims jump the gap (13..19 never tested for a bundle); TIP_CLAIM_FIRST=1 hands out 20 before the backfill's 2
  - verification: a leaf of 25 and of 20 (its start state X-1 = 19 is the tip run's), a leaf of 5, a fold [12..21]
    across X (state 11 the board pass's, 21 the tip run's), spines [1..10] and [1..25]; a receipt of 25 on another
    chain is refused
  - the handover check: the two runs agree at the board pass's newest state; a tip-run state_12 of another chain
    latches the alert
  - reorgs: the tip run's record (fork 23) is applied as `prep-tip` and acked in the TIP run's ack directory (the board
    pass's sees nothing); the board pass's own record of the same epoch number 1 is applied too (coordinator epoch 2) and
    acked in ITS directory; a tip-run record whose fork is below X - 1 while the board pass holds the block above it is
    refused, not acked; the tip run's rewind is marked done once its new branch appears
  - /api/meta's tip_run

  python3 test_reorg_tip.py                       # must PASS
  python3 test_reorg_tip.py --control-view        # verify-any reads the process's (board pass's) directories: MUST FAIL
  python3 test_reorg_tip.py --control-acks        # the tip run's acks go to the board pass's directory: MUST FAIL
  python3 test_reorg_tip.py --control-gap         # the gap is not jumped: MUST FAIL
  python3 test_reorg_tip.py --control-handover    # the handover check never fires: MUST FAIL
  python3 test_reorg_tip.py --control-belowx      # a tip-run fork below X is applied: MUST FAIL
"""
import base64, hashlib, json, os, shutil, socket, stat, subprocess, sys, tempfile, time, types, urllib.request

PATCHES = {
    "--control-view": ("    if not (LIBZYNC and TIP_FROM):\n        return None, None\n    view =",
                       "    if True:\n        return None, None\n    view ="),
    "--control-acks": ('    if TIP_FROM:\n        out.append(("prep-tip", TIP_STATE_DIR, TIP_REORG_DIR, TIP_REORG_ACK_DIR))',
                       '    if TIP_FROM:\n        out.append(("prep-tip", TIP_STATE_DIR, TIP_REORG_DIR, REORG_ACK_DIR))'),
    "--control-gap": ("        if b is None or h > b:\n            return TIP_FROM\n", "        if False:\n            return TIP_FROM\n"),
    "--control-handover": ("            hv = _handover_check()\n", "            hv = None\n"),
    "--control-belowx": ('            if src == "prep-tip" and f < TIP_FROM - 1 and', '            if False and'),
    # review of D1, each fix taken out
    "--control-fence": ("    if not (BOARD_BLOCKS_DIR or TIP_BLOCKS_DIR):\n        return None\n    run =", "    if True:\n        return None\n    run ="),
    "--control-torn": ("    if not (LIBZYNC and TIP_FROM):\n        return None\n    for h in ([lo - 1]", "    if True:\n        return None\n    for h in ([lo - 1]"),
    "--control-handover-clear": ("            elif had:\n                c.execute(\"DELETE FROM meta WHERE k='handover_alert'\")", "            elif False:\n                c.execute(\"DELETE FROM meta WHERE k='handover_alert'\")"),
    "--control-spine-view": ("        if LIBZYNC and TIP_FROM:\n            # with a tip run (D1, review LOW 8)", "        if False:\n            # with a tip run (D1, review LOW 8)"),
    "--control-orphaned": ("and _b is not None and _display(_b) == first:", "and _b is not None:"),
    "--control-window": ("            if TIP_FROM:\n                btop = _verified_top(c, \"prep\")", "            if False:\n                btop = _verified_top(c, \"prep\")"),
}
CONTROL = next((a for a in sys.argv[1:] if a in PATCHES), None)
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="reorg_tip_")
HOST = os.path.join(T, "host")
X = 20
SB, LB, AB = os.path.join(T, "board", "out"), os.path.join(T, "board", "leaves"), os.path.join(T, "acks")
ST, LT, AT = os.path.join(T, "tip", "out"), os.path.join(T, "tip", "leaves"), os.path.join(T, "acks-tip")
RB, RT = os.path.join(SB, "reorg", "records"), os.path.join(ST, "reorg", "records")
for d in (SB, LB, ST, LT, RB, RT, os.path.join(T, "tip", "blocks"), os.path.join(T, "board", "blocks")):
    os.makedirs(d)
GEN = "aa" * 32
ENV = dict(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=LB, HAZYNC_LZ_STATE_DIR=SB,
           HAZYNC_LZ_REORG_ACK_DIR=AB, HAZYNC_LZ_TIP_FROM=str(X), HAZYNC_LZ_TIP_STATE_DIR=ST, HAZYNC_LZ_TIP_BRIDGE_OUT=LT,
           HAZYNC_LZ_TIP_REORG_ACK_DIR=AT, HAZYNC_LZ_TIP_BLOCKS_DIR=os.path.join(T, "tip", "blocks"),
           HAZYNC_LZ_BLOCKS_DIR=os.path.join(T, "board", "blocks"), WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"),
           COORD_STATE=os.path.join(T, "st"), COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"),
           SEED_RANGES="0", RATE_EXEMPT="", TIP_HEIGHT="31", VERIFY_CONCURRENCY="8", GENESIS_TIP=GEN,
           TIP_FILE=os.path.join(T, "no_tip"))
os.environ.update(ENV)
with open(HOST, "w") as f:
    f.write(r'''#!/usr/bin/env python3
import hashlib, json, os, sys
a = sys.argv[1:]
assert a[0] == "verify-any", a
ce = "--check-end" in a
r = json.load(open(a[1]))
d = os.environ["HAZYNC_LZ_STATE_DIR"]
def tip(h):
    try: return bytes(json.load(open(f"{d}/state_{h}.json"))["tip_hash"]).hex()
    except Exception: return None
def link(x): return hashlib.sha256(bytes.fromhex(x)).hexdigest()
lo, hi = r["lo"], r["hi"]
if lo > 1 and tip(lo - 1) != r["in"]:
    print(f"the receipt is not anchored on the state before block {lo}", file=sys.stderr); sys.exit(1)
t = tip(hi) if ce else r["out"]
if t is None:
    print(f"the board has no state at block {hi} to check the receipt's end against", file=sys.stderr); sys.exit(1)
if t != r["out"]:
    print(f"the receipt ends on a state at block {hi} that is not the board's (its tip {r['out']}, the board's {t})", file=sys.stderr); sys.exit(1)
it = os.environ.get("GENESIS_TIP") if lo == 1 else link(r["in"])
print(f"RANGE-OK lo={lo} hi={hi} in_tip={it} out_tip={link(r['out'])} out_leaves=1 range_work=0 "
      f"in_bhash={r['in']} out_bhash={r['out']} image=x lineage=y anchored={int(lo == 1)}{' end=board' if ce else ''}")
''')
os.chmod(HOST, os.stat(HOST).st_mode | stat.S_IEXEC)

sys.path.insert(0, HERE)
src = open(os.path.join(HERE, "server.py")).read()
if CONTROL:
    a, b = PATCHES[CONTROL]
    assert src.count(a) == 1, CONTROL
    src = src.replace(a, b)
    print(f"CONTROL {CONTROL}: the checks below MUST fail")
s = types.ModuleType("server"); s.__file__ = os.path.join(HERE, "server.py"); sys.modules["server"] = s
exec(compile(src, s.__file__, "exec"), s.__dict__)
s.init_db()
if not s.HAVE_ED:
    print("SKIP: no ed25519 library"); sys.exit(0)
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization as ser

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

FORKED = {"B": ("A", 23)}
def bh(tag, h):
    while tag in FORKED and h <= FORKED[tag][1]: tag = FORKED[tag][0]
    return hashlib.sha256(f"{tag}{h}".encode()).hexdigest()
def disp(x): return bytes.fromhex(x)[::-1].hex()
def prep(d, leaves, tag, lo, hi, bundles_from=1):
    for h in range(lo, hi + 1):
        with open(os.path.join(d, f"state_{h}.json"), "w") as f:
            json.dump({"height": h, "tip_hash": list(bytes.fromhex(bh(tag, h)))}, f)
        if h >= bundles_from:
            open(os.path.join(leaves, f"leaf_{h}.bin"), "wb").write(tag.encode())
    with open(os.path.join(d, "PREP_TIP.json"), "w") as f:
        json.dump({"height": hi, "hash": disp(bh(tag, hi))}, f)
def prep_rewind(sdir, leaves, rdir, fork, epoch, tag, top):
    open(os.path.join(sdir, "REWIND"), "w").write(json.dumps({"fork": fork, "epoch": epoch}))
    for d, pre, ext in ((sdir, "state_", ".json"), (leaves, "leaf_", ".bin")):
        od = os.path.join(d, "orphaned", str(epoch)); os.makedirs(od, exist_ok=True)
        for h in range(fork + 1, top + 1):
            p = os.path.join(d, f"{pre}{h}{ext}")
            if os.path.exists(p): os.replace(p, os.path.join(od, os.path.basename(p)))
    rec = {"epoch": epoch, "fork": fork, "fork_hash": disp(bh(tag, fork)), "depth": top - fork,
           "orphaned": [[h, disp(bh(tag, h))] for h in range(fork + 1, top + 1)], "new": [], "at": time.time()}
    p = os.path.join(rdir, f"{epoch}.json")
    open(p, "w").write(json.dumps(rec))
    return hashlib.sha256(open(p, "rb").read()).hexdigest()
def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()
P_SK, P = key()
def receipt(tag, lo, hi):
    return json.dumps({"lo": lo, "hi": hi, "in": GEN if lo == 1 else bh(tag, lo - 1), "out": bh(tag, hi),
                       "n": os.urandom(8).hex()}).encode()
def body(rcpt, **kw):
    return dict(pubkey=P, sig=P_SK.sign(rcpt).hex(), receipt=base64.b64encode(rcpt).decode(), handle="P",
                _client_ip="10.1.1.1", **kw)
def submit(tag, lo, hi):
    return s.submit(body(receipt(tag, lo, hi), range=str(lo) if lo == hi else f"{lo}-{hi}"))
def q(sql, *a):
    c = s.db()
    try: return c.execute(sql, a).fetchall()
    finally: c.close()
def fresh():
    s._servable_high(force=True)
    s._board_top_cache["t"] = 0.0

prep(SB, LB, "A", 1, 12)
prep(ST, LT, "A", 1, 30, bundles_from=X)
open(os.path.join(LT, "leaf_5.bin"), "wb").write(b"sample")     # the tip run's LEAF_AT sample below X
fresh()

# ---- configuration ----
check(s._tip_config_problem() is None, "the configuration of two runs is accepted")
saved = s.TIP_REORG_ACK_DIR
s.TIP_REORG_ACK_DIR = AB
check("ack" in (s._tip_config_problem() or ""), f"one ack directory for both runs: refused ({s._tip_config_problem()})")
s.TIP_REORG_ACK_DIR = saved
saved = s.TIP_STATE_DIR
s.TIP_STATE_DIR = SB
check("two directories" in (s._tip_config_problem() or ""), "the tip run's state directory = the board pass's: refused")
s.TIP_STATE_DIR = ""
check("HAZYNC_LZ_TIP_STATE_DIR" in (s._tip_config_problem() or ""), "no tip state directory: refused")
s.TIP_STATE_DIR = saved
srv = os.path.join(HERE, "server.py")
r = subprocess.run([sys.executable, srv], env=dict(os.environ, HAZYNC_LZ_TIP_REORG_ACK_DIR=AB, COORD_PORT="1", COORD_BIND="127.0.0.1"),
                   capture_output=True, text=True, timeout=60)
check(r.returncode == 3 and "tip run" in r.stdout, f"a real start-up with one ack directory for both runs: exit 3 ({r.returncode}, {r.stdout.strip()[-90:]})")

# ---- bundles ----
check(s.bundle_path(5) == os.path.join(LB, "leaf_5.bin"), f"block 5: the board pass's bundle, not the tip run's sample ({s.bundle_path(5)})")
check(s.bundle_path(25) == os.path.join(LT, "leaf_25.bin") and s.bundle_path(X) == os.path.join(LT, f"leaf_{X}.bin"),
      "blocks 20 and 25: the tip run's bundles")
check(s.bundle_path(15) is None and not s.witness_available(15), "block 15 (the gap): no bundle")
check(s._servable_high() == 30 and s._board_servable_high() == 12, f"servable 30, the board pass's 12 ({s._servable_high()}, {s._board_servable_high()})")

# ---- claims ----
for b in range(1, 13):
    code, out = submit("A", b, b)
    if code != 200: check(False, f"board block {b}: {code} {out}")
tested = []
_wa = s.witness_available
s.witness_available = lambda b: (tested.append(b), _wa(b))[1]
code, out = s.claim({"pubkey": P, "handle": "P", "_client_ip": "10.1.1.1"})
s.witness_available = _wa
check(code == 200 and out.get("range") == str(X), f"blocks 1..12 proven: the next claim is X = {X} ({code}, {out.get('range')})")
check(not any(13 <= b < X for b in tested), f"the claim scan jumped the gap 13..19 (heights tested for a bundle: {sorted(set(tested))[-8:]})")
code, out = s.pick(None)
check(code == 200 and out.get("lo") == X + 1, f"pick after the frontier: {X + 1}, past the gap and the claim on {X} ({code}, {out})")
q_sk, qk = key()
os.remove(os.path.join(LB, "leaf_12.bin")); os.remove(os.path.join(SB, "state_12.json"))   # (put back below)
prep(SB, LB, "A", 12, 12)
c = s.db(); c.execute("DELETE FROM vranges WHERE id='2'"); c.execute("DELETE FROM ranges WHERE id='2'"); c.commit(); c.close()
s.TIP_CLAIM_FIRST = True
code, out = s.claim({"pubkey": qk, "handle": "Q", "_client_ip": "10.2.2.2"})
check(code == 200 and out.get("range") == str(X + 1), f"TIP_CLAIM_FIRST=1: a tip block ({X + 1}) before the backfill's open 2 ({code}, {out.get('range')})")
s.TIP_CLAIM_FIRST = False
code, out = s.claim({"pubkey": key()[1], "handle": "R", "_client_ip": "10.3.3.3"})
check(code == 200 and out.get("range") == "2", f"TIP_CLAIM_FIRST off: the backfill's 2 first ({code}, {out.get('range')})")
code, out = submit("A", 2, 2)

# ---- verification ----
code, out = submit("A", 25, 25)
check(code == 200, f"a leaf of 25 (states 24, 25: the tip run's) verifies ({code}, {str(out)[:100]})")
code, out = submit("A", X, X)
check(code == 200, f"a leaf of X = {X} (its start state {X - 1} is the tip run's) verifies ({code}, {str(out)[:100]})")
code, out = s.submit(body(json.dumps({"lo": 26, "hi": 26, "in": bh("A", 25), "out": bh("Z", 26), "n": "x"}).encode(), range="26"))
check(code != 200 and not q("SELECT 1 FROM vranges WHERE id='26'"), f"a receipt of 26 on another chain: refused ({code})")
for h in (10, 25):
    code, out = s.submit_spine(body(receipt("A", 1, h)))
    check(code == 200, f"the spine [1..{h}] verifies ({code}, {str(out)[:90]})")
check(not [n for n in os.listdir(os.path.join(T, "st")) if n.endswith(".view")], "no verification view left behind")

# a fold across X once the board pass has caught up to X - 1: its start state the board pass's, its end the tip run's
prep(SB, LB, "A", 13, X - 1)
fresh()
for b in range(13, X):
    submit("A", b, b)
code, out = submit("A", 17, 20)
check(code == 200, f"a fold [17..20] across X (state 16 the board pass's, 20 the tip run's) verifies ({code}, {str(out)[:100]})")

# ---- the handover check ----
fresh()
top = s._board_state_top()
check(top == X - 1 and s._handover_check() is None, f"the two runs agree at the board pass's newest state ({top})")
good = open(os.path.join(ST, f"state_{top}.json")).read()
with open(os.path.join(ST, f"state_{top}.json"), "w") as f:
    json.dump({"height": top, "tip_hash": list(bytes.fromhex(bh("Z", top)))}, f)
fresh()
s.reorg_backstop()
al = q("SELECT v FROM meta WHERE k='handover_alert'")
check(bool(al) and f"disagree on state_{top}.json" in al[0][0], f"a tip-run state_{top} of another chain sets the handover alert ({al and al[0][0][:90]})")
open(os.path.join(ST, f"state_{top}.json"), "w").write(good)
good = open(os.path.join(ST, "state_12.json")).read()
c = s.db(); c.execute("DELETE FROM meta WHERE k='reorg_alert'"); c.commit(); c.close()
fresh()
s.reorg_backstop()
check(not q("SELECT v FROM meta WHERE k='reorg_alert'") and not q("SELECT v FROM meta WHERE k='handover_alert'"), "agreeing again: no alert")

# ---- reorgs ----
for b in range(22, 28):
    submit("A", b, b)
sha_t = prep_rewind(ST, LT, RT, 23, 1, "A", 30)
applied = s.reorg_poll()
os.remove(os.path.join(ST, "REWIND"))
try: ack_t = json.load(open(os.path.join(AT, "1.ack")))
except Exception: ack_t = {}
check(applied == [1] and ack_t.get("record_sha256") == sha_t, f"the tip run's record (fork 23) applied and acked in ITS ack directory ({applied}, {ack_t})")
check(not os.path.exists(os.path.join(AB, "1.ack")), "nothing in the board pass's ack directory")
check(q("SELECT src FROM reorgs WHERE epoch=1")[0][0] == "prep-tip" and not q("SELECT 1 FROM vranges WHERE hi > 23"),
      "recorded as prep-tip; no verified range above 23")
prep(ST, LT, "B", 24, 30, bundles_from=X)
s.reorg_poll()
check(q("SELECT state FROM reorgs WHERE epoch=1")[0][0] == "done", "the tip run's new branch appears: the rewind is done")
code, out = submit("B", 24, 24)
check(code == 200, f"B's block 24 (tip run) verifies on the new branch ({code}, {str(out)[:80]})")
# the nightly proof check (check-proofs.py) re-verifies every stored proof against the run that holds its heights; without
# the tip run's variables it cannot (the board pass has no state from X on): its control, in place
ck = [sys.executable, os.path.join(HERE, "check-proofs.py"), "--db", os.environ["COORD_DB"], "--proofs", os.path.join(T, "p"),
      "--host", HOST, "--min-age", "0"]
r = subprocess.run(ck, env=dict(os.environ), capture_output=True, text=True, timeout=120)
check(r.returncode == 0 and "with problems        : 0" in r.stdout, f"check-proofs with the tip run: every proof holds ({r.returncode}, {r.stdout.strip().splitlines()[-1][:80]})")
r = subprocess.run(ck, env=dict(os.environ, HAZYNC_LZ_TIP_FROM=""), capture_output=True, text=True, timeout=120)
check(r.returncode == 1 and "proof_24.bin" in r.stdout, f"check-proofs without the tip run's variables: the tip run's proofs fail (control; {r.returncode})")
# its X must be the coordinator's (review, LOW 11): a stand-in /api/meta saying 21
import http.server, threading
class Meta(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = json.dumps({"tip_run": {"from": X + 1}}).encode(); self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
msv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Meta); threading.Thread(target=msv.serve_forever, daemon=True).start()
r = subprocess.run(ck, env=dict(os.environ, CHECK_PROOFS_COORD=f"http://127.0.0.1:{msv.server_address[1]}"), capture_output=True, text=True, timeout=120)
check(r.returncode == 2 and "starts at 21" in r.stdout, f"check-proofs with an X other than the coordinator's: exit 2, nothing checked ({r.returncode})")
msv.shutdown()
# the board pass's own record, epoch 1 in ITS numbering
sha_b = prep_rewind(SB, LB, RB, 11, 1, "A", 12)
applied = s.reorg_poll()
os.remove(os.path.join(SB, "REWIND"))
try: ack_b = json.load(open(os.path.join(AB, "1.ack")))
except Exception: ack_b = {}
check(applied == [2] and ack_b.get("record_sha256") == sha_b and ack_b.get("coord_epoch") == 2,
      f"the board pass's record of the same number applied as epoch 2, acked in ITS directory ({applied}, {ack_b})")
try: still = json.load(open(os.path.join(AT, "1.ack"))).get("record_sha256") == sha_t
except Exception: still = False
check(still, "the tip run's ack 1 untouched")
prep(SB, LB, "A", 12, 12)
# a tip-run fork below X - 1 while the board pass holds the block above it (the tip run's files above 10 moved aside, as
# its rewind does; put back afterwards)
open(os.path.join(SB, "state_12.json"), "w").write(good)
aside = os.path.join(T, "aside"); os.makedirs(aside)
for h in range(11, 31):
    os.replace(os.path.join(ST, f"state_{h}.json"), os.path.join(aside, f"state_{h}.json"))
open(os.path.join(ST, "REWIND"), "w").write(json.dumps({"fork": 10, "epoch": 2}))
rec = {"epoch": 2, "fork": 10, "fork_hash": disp(bh("A", 10)), "depth": 20, "orphaned": [[h, disp(bh("B", h))] for h in range(11, 31)],
       "new": [], "at": time.time()}
open(os.path.join(RT, "2.json"), "w").write(json.dumps(rec))
applied = s.reorg_poll()
check(applied == [] and not os.path.exists(os.path.join(AT, "2.ack")) and len(q("SELECT 1 FROM reorgs")) == 2,
      f"a tip-run fork (10) below X - 1 with the board pass holding block 11: refused, not acked ({applied})")
os.remove(os.path.join(ST, "REWIND")); os.remove(os.path.join(RT, "2.json"))
for h in range(11, 31):
    os.replace(os.path.join(aside, f"state_{h}.json"), os.path.join(ST, f"state_{h}.json"))

# ---- /api/meta ----
s._sf.clear()
fresh()
tm = s.tip_run_meta() or {}
check(tm.get("from") == X and tm.get("servable_high") == 30 and tm.get("board_servable_high") == X - 1
      and (tm.get("prep_tip") or {}).get("height") == 30, f"/api/meta tip_run: {tm}")

# ---- the stall check (hazync-lz-check-reorg.py with CHECK_TIP_RUN_PREP_TIP) ----
ck = os.path.join(HERE, "deploy/libzync/hazync-lz-check-reorg.py")
c = s.db(); c.execute("INSERT OR REPLACE INTO meta(k,v) VALUES('reorg_backstop_at',?)", (str(time.time()),)); c.execute("DELETE FROM meta WHERE k IN ('reorg_alert','handover_alert')"); c.commit(); c.close()
nt = os.path.join(T, "node_tip"); open(nt, "w").write("40\n")
pt, sf = os.path.join(ST, "PREP_TIP.json"), os.path.join(T, "check-tip-run.json")
cenv = dict(os.environ, CHECK_TIP_RUN_PREP_TIP=pt, NODE_TIP_FILE=nt, CHECK_TIP_RUN_STATE=sf, CHECK_TIP_RUN_STALE_S="2")
def stall_check(**kw): return subprocess.run([sys.executable, ck], env=dict(cenv, **kw), capture_output=True, text=True, timeout=60)
os.utime(pt, (time.time() - 7200, time.time() - 7200))
r = stall_check()
check(r.returncode == 0, f"the node gets ahead after a quiet 2 h (PREP_TIP.json old): no page yet ({r.returncode}: {r.stdout.strip()[-60:]})")
time.sleep(2.5)
r = stall_check()
check(r.returncode == 1 and "TIP RUN STALLED" in r.stdout, f"the node ahead for longer than the limit, the tip run at 30: pages ({r.returncode})")
r = stall_check(CHECK_TIP_RUN_LEAVES=LT, CHECK_TIP_RUN_LEAF_WINDOW="3")
check(r.returncode == 1 and "paused by its leaf window" in r.stdout, f"... and says when its leaf window paused it ({r.stdout.strip()[-80:]})")
open(nt, "w").write("30\n")
r = stall_check()
check(r.returncode == 0, f"at the node's tip and waiting for a block: no page ({r.returncode})")
os.rename(pt, pt + ".away")
r = stall_check()
check(r.returncode == 0 and "not checked" in r.stdout, f"no PREP_TIP.json yet (before the restart): said, not paged ({r.returncode})")
os.rename(pt + ".away", pt)

# ---- review of D1 ----
q_sk, qk = key()
def qbody(rcpt, rid): return dict(pubkey=qk, sig=q_sk.sign(rcpt).hex(), receipt=base64.b64encode(rcpt).decode(), handle="Q", _client_ip="10.9.9.9", range=rid)
def att(rid):
    a = q("SELECT attempts FROM ranges WHERE id=?", rid)
    return a[0][0] if a else 0
# 2. a REORG marker in the tip run's block directory (automatic rewinds off: prep stopped, nothing moved yet)
open(os.path.join(T, "tip", "blocks", "REORG"), "w").write(json.dumps({"fork": 27, "class": "auto"}))
s._fence_cache["t"] = 0.0
code, out = s.submit(qbody(receipt("B", 28, 28), "28"))
check(code == 409 and out.get("reorged") and not att("28") and not q("SELECT 1 FROM vranges WHERE id='28'"),
      f"2. a tip-run REORG marker at 27: a proof of 28 is refused 409 reorged, not counted ({code})")
check(s.bundle_path(28) is None and s.bundle_path(27) == os.path.join(LT, "leaf_27.bin") and s.lz_state_tip(28) is None,
      "2. the bundle of 28 is no longer served, 27's is; state 28 reads as none (the commit check refuses it)")
code, out = s.submit(qbody(receipt("B", 25, 25), "25"))
check(code == 200, f"2. below the fence, 25: verified as before ({code}, {str(out)[:60]})")
os.remove(os.path.join(T, "tip", "blocks", "REORG"))
open(os.path.join(T, "tip", "blocks", "REORG_TOO_DEEP"), "w").write(json.dumps({"fork": None}))
s._fence_cache["t"] = 0.0
check(s.bundle_path(X) is None and s.bundle_path(X - 1) is not None, "2. REORG_TOO_DEEP with no fork fences the whole tip run (from X - 1)")
os.remove(os.path.join(T, "tip", "blocks", "REORG_TOO_DEEP"))
s._fence_cache["t"] = 0.0
code, out = s.submit(qbody(receipt("B", 28, 28), "28"))
check(code == 200, f"2. the marker gone: 28 verified ({code})")
# 7. a missing, empty or torn state at lo-1 or hi: 503, not counted, the verifier not run
for kind, data in (("empty", b""), ("torn", b'{"height": 2')):
    path = os.path.join(ST, "state_29.json"); good29 = open(path, "rb").read()
    open(path, "wb").write(data)
    code, out = s.submit(qbody(receipt("B", 30, 30), "30"))
    check(code == 503 and not att("30") and not q("SELECT 1 FROM vranges WHERE id='30'"), f"7. state 29 {kind}: a proof of 30 is answered 503, not counted ({code})")
    open(path, "wb").write(good29)
code, out = s.submit(qbody(receipt("B", 30, 30), "30"))
check(code == 200, f"7. state 29 back: 30 verified ({code})")
# a leaf of X - 1: the board pass's bundle, its start state X - 2 the board pass's, its end state X - 1 the tip run's
c = s.db(); c.execute("DELETE FROM vranges WHERE id=?", (str(X - 1),)); c.execute("DELETE FROM ranges WHERE id=?", (str(X - 1),)); c.commit(); c.close()
code, out = s.submit(qbody(receipt("A", X - 1, X - 1), str(X - 1)))
check(code == 200, f"a leaf of X - 1 = {X - 1} verifies (state {X - 2} the board pass's, {X - 1} the tip run's) ({code}, {str(out)[:60]})")
# 6. the handover alert clears itself, and does not block the other latch
top = s._board_state_top() if (s._board_top_cache.update(t=0.0) or True) else None
gtop = open(os.path.join(ST, f"state_{top}.json")).read()
json.dump({"height": top, "tip_hash": list(bytes.fromhex(bh("Z", top)))}, open(os.path.join(ST, f"state_{top}.json"), "w"))
s._board_top_cache["t"] = 0.0
s.reorg_backstop()
check(bool(q("SELECT 1 FROM meta WHERE k='handover_alert'")), "6. the runs disagree: handover_alert set")
r = stall_check(CHECK_TIP_RUN_PREP_TIP="")
check(r.returncode == 1 and "HANDOVER ALERT" in r.stdout, f"6. hazync-lz-check-reorg pages on it ({r.returncode})")
open(os.path.join(ST, f"state_{top}.json"), "w").write(gtop)
s._board_top_cache["t"] = 0.0
s.reorg_backstop()
check(not q("SELECT 1 FROM meta WHERE k='handover_alert'") and not q("SELECT 1 FROM meta WHERE k='reorg_alert'"),
      "6. agreeing again: the handover alert cleared by itself, no reorg alert latched")
# 11. the backstop's window per run: a backfill range off the board pass's chain is caught although tip blocks are
# verified far above it (REORG_AUTO_MAX_DEPTH counted from the tip would leave the backfill out)
for b in range(12, X - 1):
    s.submit(qbody(receipt("A", b, b), str(b)))
s.REORG_AUTO_MAX_DEPTH = 5
g5 = open(os.path.join(SB, "state_5.json")).read()
json.dump({"height": 5, "tip_hash": list(bytes.fromhex(bh("Z", 5)))}, open(os.path.join(SB, "state_5.json"), "w"))
s.reorg_backstop()
al = q("SELECT v FROM meta WHERE k='reorg_alert'")
check(not al, "11. a backfill state off the chain BELOW the board pass's window (5 < 19 - 5): not compared")
open(os.path.join(SB, "state_5.json"), "w").write(g5)
g16 = open(os.path.join(SB, "state_16.json")).read()
json.dump({"height": 16, "tip_hash": list(bytes.fromhex(bh("Z", 16)))}, open(os.path.join(SB, "state_16.json"), "w"))
s.reorg_backstop()
al = q("SELECT v FROM meta WHERE k='reorg_alert'")
check(bool(al) and "16" in al[0][0], f"11. a backfill range within the board pass's own window (16), tip blocks verified up to {s._verified_top(s.db(), None)}: latched ({al and al[0][0][:70]})")
open(os.path.join(SB, "state_16.json"), "w").write(g16)
c = s.db(); c.execute("DELETE FROM meta WHERE k='reorg_alert'"); c.commit(); c.close()
s.REORG_AUTO_MAX_DEPTH = 2016
# 8. the spine is checked against the run that holds its end: a board-pass state that differs from the tip run's at 10
g12t = open(os.path.join(ST, "state_12.json")).read()
json.dump({"height": 12, "tip_hash": list(bytes.fromhex(bh("Z", 12)))}, open(os.path.join(ST, "state_12.json"), "w"))
open(os.path.join(LT, "leaf_13.bin"), "wb").write(b"a tip-run sample bundle the host would prefer")
code, out = s.submit_spine(body(receipt("A", 1, 12)))
check(code == 200, f"8. a spine [1..12] on the board pass's chain verifies against the board pass's state 12 (not the tip run's) ({code}, {str(out)[:70]})")
open(os.path.join(ST, "state_12.json"), "w").write(g12t); os.remove(os.path.join(LT, "leaf_13.bin"))
# sweep: a view a crash left behind goes at start-up
os.makedirs(os.path.join(T, "st", "in_x.bin.view"), exist_ok=True)
s.sweep_views()
check(not os.path.exists(os.path.join(T, "st", "in_x.bin.view")), "11. a verification view a crash left is swept at start-up")
# 11. the board pass's directory unreadable for a moment: the gap jump keeps its last board high
hb = s._board_servable_high()
s.BRIDGE_DIR, keep = os.path.join(T, "nowhere"), s.BRIDGE_DIR
s._servable_high(force=True)
check(s._board_servable_high() == hb and s._skip_gap(5) == 5, f"11. the board pass's bundles unreadable: its high stays {hb}, 5 is not jumped")
s.BRIDGE_DIR = keep; s._servable_high(force=True)
# 9. a tip-run fork below X - 1 is refused only when the board pass holds the ORPHANED block above it
open(os.path.join(ST, "REWIND"), "w").write(json.dumps({"fork": 10, "epoch": 3}))
aside = os.path.join(T, "aside9"); os.makedirs(aside)
for h in range(11, 31):
    if os.path.exists(os.path.join(ST, f"state_{h}.json")): os.replace(os.path.join(ST, f"state_{h}.json"), os.path.join(aside, f"state_{h}.json"))
rec = {"epoch": 3, "fork": 10, "fork_hash": disp(bh("A", 10)), "depth": 20, "orphaned": [[h, disp(bh("Y", h))] for h in range(11, 31)], "new": [], "at": time.time()}
open(os.path.join(RT, "3.json"), "w").write(json.dumps(rec))
applied = s.reorg_poll()
check(applied and os.path.exists(os.path.join(AT, "3.ack")), f"9. a tip-run fork below X - 1 whose orphaned block 11 the board pass does NOT hold (it holds another): applied ({applied})")
os.remove(os.path.join(ST, "REWIND"))
for h in range(11, 31):
    if os.path.exists(os.path.join(aside, f"state_{h}.json")): os.replace(os.path.join(aside, f"state_{h}.json"), os.path.join(ST, f"state_{h}.json"))
# 5. rolling back (one run again) after tip blocks were verified: without the documented --reorg-to X - 1 first, the
# backstop latches (the tip run's states are no longer read); with it, nothing is left above X - 1
for b in range(X, X + 4): s.submit(qbody(receipt("A" if b <= 23 else "B", b, b), str(b)))
saved = (s.TIP_FROM, s.TIP_STATE_DIR)
s.TIP_FROM, s.TIP_STATE_DIR = 0, ""
s.reorg_backstop()
check(bool(q("SELECT 1 FROM meta WHERE k='reorg_alert'")), "5. the tip-run drop-in deleted with tip blocks verified: the backstop latches (why the RUNBOOK says --reorg-to first)")
c = s.db(); c.execute("DELETE FROM meta WHERE k='reorg_alert'"); c.commit(); c.close()
s.reorg_rewind(X - 1, "manual")
s.reorg_backstop()
check(not q("SELECT 1 FROM meta WHERE k='reorg_alert'") and not q("SELECT 1 FROM vranges WHERE hi >= ?", X),
      "5. rolled back by the RUNBOOK's order (--reorg-to X - 1, then the drop-in): no range above X - 1, no alert")
s.TIP_FROM, s.TIP_STATE_DIR = saved

shutil.rmtree(T, ignore_errors=True)
if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail under {CONTROL}, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: nothing failed under {CONTROL} -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
