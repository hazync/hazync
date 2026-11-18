#!/usr/bin/env python3
"""The coordinator's side of a reorg (docs/libzync/REORG.md §4, §5 steps 4-6, §7; phase P4), with a fake verifier.

The fake `host verify-any` reads a receipt that says which chain it was made on (the block hashes it starts and ends on)
and checks it against prep's state files the way lz_coord.rs does (the state before lo, and --check-end: the board's
state at hi), BEFORE an optional delay: a verification that straddles a rewind read the old files. Prep is simulated:
its state files (tip_hash per height), leaf bundles, and the rewind's file moves and record.

Old chain A: blocks 1..25 prepped. Key P proves 1..20, a fold [17..20], the spine reaches 10, 16 and 20 (kept heads), a
sponsorship of 19..20 is proven; key Q claims block 21; Q's proof of 22 is verifying (slowly) when prep rewinds to F = 17
(depth 8). Then:
  - nothing is acked while prep's state 18 is still A's (its files have not moved, §5 step 3)
  - the record applied: vranges above 17 moved to vranges_orphaned, ranges rows above 17 to ranges_orphaned, the
    frontier at 17, the spine back at the kept head 16 (20 moved to spine/orphaned/1), the proofs above 17 in
    proofs/orphaned/1/ (and served there), the sponsorship back to `proving`, P's credit kept (`orphaned` 3), the ack
    with the record's sha256 in the ack directory, /api/meta's reorg `rewinding`, block 18's page lists the orphaned
    proof
  - Q's straddling proof of 22: 409 `reorged`, not credited; Q's beat on the voided claim of 21: 409
  - the record polled again: not applied twice; its ack removed: written again
  - an orphaned-branch proof of 19 submitted after the rewind: 409 `reorged`, not counted against block 19 (no park)
  - the new chain B (18..22) appears: the rewind is `done`; B's block 18 proven under the SAME range id; the orphaned
    proof of 18 is still served; a spine head of A's chain ending at 20 is refused (409 `reorged`)
  - a second reorg (fork 19) while a spine head of B is verifying: 409 `reorged`, the spine stays at 16
  - a third record applied at START-UP, before the port answers (a real server process): epoch 3, fork 20
  - `server.py --reorg-to 15`: a dry run changes nothing; with --yes, epoch 4 (manual), the frontier at 15

  python3 test_reorg.py                    # must PASS
  python3 test_reorg.py --control          # no check at commit: the straddling proof is credited; MUST FAIL
  python3 test_reorg.py --control-keep     # the orphaned rows stay in vranges: the frontier stays on A; MUST FAIL
"""
import base64, hashlib, json, os, shutil, socket, stat, subprocess, sys, tempfile, threading, time, types, urllib.request

PATCHES = {
    "--control": ("            _why = reorg_refusal(c, int(meta.get(\"lo\", r[\"lo\"])), int(meta.get(\"hi\", r[\"hi\"])), meta, _ep0)\n",
                  "            _why = None\n"),
    "--control-keep": ('            c.execute("DELETE FROM vranges WHERE hi > ?", (fork,))\n', "            pass\n"),
}
CONTROL = next((a for a in sys.argv[1:] if a in PATCHES), None)
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="reorg_")
HOST = os.path.join(T, "host")
STATE, LEAVES, ACKS = os.path.join(T, "out"), os.path.join(T, "leaves"), os.path.join(T, "acks")
RECORDS = os.path.join(STATE, "reorg", "records")
for d in (STATE, LEAVES, RECORDS):
    os.makedirs(d)
GEN = "aa" * 32
ENV = dict(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=LEAVES, HAZYNC_LZ_STATE_DIR=STATE,
           HAZYNC_LZ_REORG_ACK_DIR=ACKS, WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"),
           COORD_STATE=os.path.join(T, "st"), COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"),
           SEED_RANGES="0", RATE_EXEMPT="", TIP_HEIGHT="30", VERIFY_CONCURRENCY="8", GENESIS_TIP=GEN,
           TIP_FILE=os.path.join(T, "no_tip"))
os.environ.update(ENV)
with open(HOST, "w") as f:
    f.write(r'''#!/usr/bin/env python3
import hashlib, json, os, sys, time
a = sys.argv[1:]
assert a[0] == "verify-any" and "--check-end" in a, a
r = json.load(open(a[1]))
d = os.environ["HAZYNC_LZ_STATE_DIR"]
def tip(h):
    try: return bytes(json.load(open(f"{d}/state_{h}.json"))["tip_hash"]).hex()
    except Exception: return None
def link(x): return hashlib.sha256(bytes.fromhex(x)).hexdigest()
lo, hi = r["lo"], r["hi"]
if lo > 1 and tip(lo - 1) != r["in"]:
    print(f"the receipt is not anchored on the state before block {lo} (anchor_id {link(r['in'])}, that state's digest "
          f"{link(tip(lo - 1) or '00')})", file=sys.stderr); sys.exit(1)
t = tip(hi)
if t is None:
    print(f"the board has no state at block {hi} to check the receipt's end against", file=sys.stderr); sys.exit(1)
if t != r["out"]:
    # lz_coord.rs's words, with its two hashes: the coordinator's note keeps only the tail, which they push the
    # reorg-shaped words out of (found by reorg_e2e.py)
    print(f"the receipt ends on a state at block {hi} that is not the board's (its tip {r['out']}, the board's {t}): "
          f"not Bitcoin's chain", file=sys.stderr); sys.exit(1)
time.sleep(r.get("delay", 0))
it = os.environ.get("GENESIS_TIP") if lo == 1 else link(r["in"])
print(f"RANGE-OK lo={lo} hi={hi} in_tip={it} out_tip={link(r['out'])} out_leaves=1 range_work=0 "
      f"in_bhash={r['in']} out_bhash={r['out']} image=x lineage=y anchored={int(lo == 1)} end=board")
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

# ---- prep, simulated ----
FORKED = {"B": ("A", 17), "C": ("B", 19)}
def bh(tag, h):
    """Block h's hash on chain `tag` (internal order): chain B is A up to 17, chain C is B up to 19."""
    while tag in FORKED and h <= FORKED[tag][1]: tag = FORKED[tag][0]
    return hashlib.sha256(f"{tag}{h}".encode()).hexdigest()
def disp(x): return bytes.fromhex(x)[::-1].hex()
def prep(tag, lo, hi):
    for h in range(lo, hi + 1):
        with open(os.path.join(STATE, f"state_{h}.json"), "w") as f:
            json.dump({"height": h, "tip_hash": list(bytes.fromhex(bh(tag, h)))}, f)
        open(os.path.join(LEAVES, f"leaf_{h}.bin"), "wb").write(tag.encode())
def prep_rewind(fork, epoch, tag, top, move=True, name=None):
    """§5 steps 3-4: the files above the fork aside, then the record."""
    if move:
        for d, pre, ext in ((STATE, "state_", ".json"), (LEAVES, "leaf_", ".bin")):
            od = os.path.join(d, "orphaned", str(epoch)); os.makedirs(od, exist_ok=True)
            for h in range(fork + 1, top + 1):
                p = os.path.join(d, f"{pre}{h}{ext}")
                if os.path.exists(p): os.replace(p, os.path.join(od, os.path.basename(p)))
    rec = {"epoch": epoch, "fork": fork, "fork_hash": disp(bh(tag, fork)), "depth": top - fork,
           "orphaned": [[h, disp(bh(tag, h))] for h in range(fork + 1, top + 1)], "new": [], "at": time.time()}
    p = os.path.join(RECORDS, f"{name or epoch}.json")
    open(p, "w").write(json.dumps(rec))
    return hashlib.sha256(open(p, "rb").read()).hexdigest()

# ---- the board ----
def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()
P_SK, P = key()
Q_SK, Q = key()
def receipt(tag, lo, hi, delay=0):
    return json.dumps({"lo": lo, "hi": hi, "in": GEN if lo == 1 else bh(tag, lo - 1), "out": bh(tag, hi),
                       "delay": delay, "n": os.urandom(8).hex()}).encode()
def body(sk, pk, rcpt, **kw):
    return dict(pubkey=pk, sig=sk.sign(rcpt).hex(), receipt=base64.b64encode(rcpt).decode(),
                handle="P" if pk == P else "Q", _client_ip="10.1.1.1", **kw)
def submit(sk, pk, tag, lo, hi, delay=0):
    rid = str(lo) if lo == hi else f"{lo}-{hi}"
    return s.submit(body(sk, pk, receipt(tag, lo, hi, delay), range=rid))
def q(sql, *a):
    c = s.db()
    try: return c.execute(sql, a).fetchall()
    finally: c.close()
def frontier():
    s._frontier_invalidate()
    return s.frontier_hi()

prep("A", 1, 25)
for b in range(1, 21):
    code, out = submit(P_SK, P, "A", b, b)
    if code != 200: check(False, f"block {b} on A: {code} {out}")
code, out = submit(P_SK, P, "A", 17, 20)
check(code == 200, f"A: blocks 1..20 and the fold [17..20] proven by P ({code})")
for h in (10, 16, 20):
    code, out = s.submit_spine(body(P_SK, P, receipt("A", 1, h)))
    check(code == 200, f"A: the spine to {h} ({code}, {str(out)[:80]})")
check(s.spine_heads() == [10, 16, 20], f"heads kept: {s.spine_heads()}")
c = s.db()
c.execute("INSERT INTO sponsorships(lo,hi,name,status,created_at,paid_sats,paid_at,proven_at) VALUES(19,20,'S','proven',?,1000,?,?)",
          (time.time(), time.time(), time.time()))
c.commit(); c.close()
code, out = s.claim({"pubkey": Q, "handle": "Q", "_client_ip": "10.2.2.2"})
check(code == 200 and out.get("range") == "21", f"Q claims 21 ({code}, {out.get('range')})")
check(frontier() == 20, f"A: frontier 20 ({frontier()})")

# Q's proof of 22 (on A) is verifying, slowly, while prep rewinds
straddle = {}
th = threading.Thread(target=lambda: straddle.update(r=submit(Q_SK, Q, "A", 22, 22, delay=3)))
th.start(); time.sleep(1)

# §5 step 3 not done yet: a record whose orphaned states are still in place is not applied
sha1 = prep_rewind(17, 1, "A", 25, move=False)
s.reorg_poll()
check(not os.path.exists(os.path.join(ACKS, "1.ack")) and frontier() == 20,
      "a record while prep's state 18 is still A's: not applied, not acked")
# now the moves, then the same record
sha1 = prep_rewind(17, 1, "A", 25)
applied = s.reorg_poll()
th.join()
check(applied == [1], f"the record applied: epoch {applied}")
try: ack = json.load(open(os.path.join(ACKS, "1.ack")))
except Exception: ack = {}
check(ack.get("record_sha256") == sha1 and ack.get("coord_epoch") == 1, f"acked with the record's sha256 ({ack})")
check(frontier() == 17, f"the frontier back at 17 ({frontier()})")
check(not q("SELECT 1 FROM vranges WHERE hi > 17"), "no verified range above 17")
orph = sorted((r[0], r[1]) for r in q("SELECT lo, hi FROM vranges_orphaned WHERE epoch=1"))
check(orph == [(17, 20), (18, 18), (19, 19), (20, 20)], f"vranges_orphaned: {orph}")
check(not q("SELECT 1 FROM ranges WHERE hi > 17") and q("SELECT 1 FROM ranges_orphaned WHERE id='21' AND status='claimed'"),
      "ranges above 17 moved to ranges_orphaned, Q's claim on 21 with them")
check(s.spine_head()["hi"] == 16 and s.spine_heads() == [10, 16]
      and os.path.exists(os.path.join(T, "sp", "orphaned", "1", "spine_20.bin")),
      f"the spine back at the kept head 16, head 20 in spine/orphaned/1 ({s.spine_head()['hi']}, {s.spine_heads()})")
check(sorted(os.listdir(os.path.join(T, "p", "orphaned", "1"))) == ["proof_17-20.bin", "proof_18.bin", "proof_19.bin", "proof_20.bin"]
      and not os.path.exists(os.path.join(T, "p", "proof_18.bin")),
      f"proofs above 17 in proofs/orphaned/1 ({sorted(os.listdir(os.path.join(T, 'p', 'orphaned', '1')))})")
check(q("SELECT status FROM sponsorships")[0][0] == "proving", "the sponsorship of 19..20 back to proving (held again)")
cp = s.contributions_by_pubkey().get(P, {})
check(cp.get("proved") == 20 and cp.get("orphaned") == 3, f"P keeps the credit: proved 20, of which orphaned 3 ({cp})")
check(s.proven_count() == 17, f"the board's proven count excludes them ({s.proven_count()})")
m = s.reorg_meta() or {}
check(m.get("epoch") == 1 and m.get("fork") == 17 and m.get("depth") == 8 and m.get("state") == "rewinding"
      and m.get("orphaned", [[0]])[0][0] == 18, f"/api/meta reorg: {dict((k, m.get(k)) for k in ('epoch', 'fork', 'depth', 'state'))}")
code, bd = s.block_detail(18)
check(code == 200 and bd.get("orphaned") and bd["orphaned"][0]["proof"] == "/api/proof/orphaned/1/18"
      and bd["orphaned"][0]["block_hash"] == disp(bh("A", 18)) and not bd["proofs"],
      f"block 18's page: no proof, the orphaned one listed ({bd.get('orphaned')})")
code, out = straddle.get("r", (None, None))
check(code == 409 and out.get("reorged") and not q("SELECT 1 FROM vranges WHERE id='22'"),
      f"Q's proof of 22, verified across the rewind: 409 reorged, not credited ({code}, {str(out)[:90]})")
ts = int(time.time())
code, out = s.beat({"range": "21", "pubkey": Q, "ts": ts, "sig": Q_SK.sign(f"21:{ts}".encode()).hex()})
check(code == 409, f"Q's beat on the voided claim of 21: 409 ({code}, {str(out)[:60]})")
check(s.reorg_poll() == [] and len(q("SELECT 1 FROM reorgs")) == 1, "polled again: not applied twice")
os.remove(os.path.join(ACKS, "1.ack"))
s.reorg_poll()
check(os.path.exists(os.path.join(ACKS, "1.ack")), "its ack removed: written again")
code, out = submit(P_SK, P, "A", 19, 19)
att = q("SELECT attempts FROM ranges WHERE id='19'")
check(code == 409 and out.get("reorged") and (not att or not att[0][0]),
      f"A's proof of 19 after the rewind: 409 reorged, not counted against block 19 ({code}, attempts {att and att[0][0]})")

# the new branch
prep("B", 18, 22)
s.reorg_poll()
check((s.reorg_meta() or {}).get("state") == "done", "chain B's states above 17 appear: the rewind is done")
code, out = submit(Q_SK, Q, "B", 18, 18)
check(code == 200 and q("SELECT pubkey FROM vranges WHERE id='18'")[0][0] == Q and frontier() == 18,
      f"B's block 18 proven by Q under the same range id ({code}, frontier {frontier()})")
check(open(os.path.join(T, "p", "orphaned", "1", "proof_18.bin"), "rb").read() != open(os.path.join(T, "p", "proof_18.bin"), "rb").read(),
      "the orphaned proof of 18 kept beside the new one")
for b in (19, 20, 21):
    submit(Q_SK, Q, "B", b, b)
check(frontier() == 21, f"B proven to 21 ({frontier()})")

# a second reorg (fork 19, B's 20..22 orphaned) while B's spine head [1..21] is verifying
sp = {}
th = threading.Thread(target=lambda: sp.update(r=s.submit_spine(body(Q_SK, Q, receipt("B", 1, 21, delay=3)))))
th.start(); time.sleep(1)
prep_rewind(19, 2, "B", 22)
applied = s.reorg_poll()
th.join()
code, out = sp.get("r", (None, None))
check(applied == [2] and code == 409 and out.get("reorged") and s.spine_head()["hi"] == 16 and frontier() == 19,
      f"a spine head verified across the second rewind (fork 19): 409 reorged, the spine stays at 16 ({code}, {str(out)[:80]})")
prep("C", 20, 22)
for b in (20, 21):
    submit(P_SK, P, "C", b, b)
check(frontier() == 21, f"C proven to 21 ({frontier()})")

# start-up: a third rewind (fork 20, C's 21..22 orphaned) applied before the port answers
sha3 = prep_rewind(20, 3, "C", 22)
port = (lambda so: (so.bind(("127.0.0.1", 0)), so.getsockname()[1], so.close())[1])(socket.socket())
srv = os.path.join(T, "server.py")
open(srv, "w").write(src)
env = dict(os.environ, COORD_PORT=str(port), COORD_BIND="127.0.0.1", COORD_ALLOW_MOCK="", REORG_POLL_S="3600",
           PYTHONUNBUFFERED="1")
proc = subprocess.Popen([sys.executable, srv], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
meta = None
for _ in range(100):
    try:
        meta = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/api/meta", timeout=2)); break
    except Exception:
        time.sleep(0.1)
proc.terminate()
log = proc.communicate()[0]
i, j = log.find("applied at start-up: epoch(s) [3]"), log.find(f":{port}  db=")
check(meta is not None and (meta.get("reorg") or {}).get("epoch") == 3 and meta.get("frontier") == 20
      and os.path.exists(os.path.join(ACKS, "3.ack")) and 0 <= i < j,
      f"a record applied at START-UP, before serving: /api/meta reorg epoch {((meta or {}).get('reorg') or {}).get('epoch')}, frontier {(meta or {}).get('frontier')}")

# the runbook's --reorg-to (REORG.md §11 step 4)
r = subprocess.run([sys.executable, srv, "--reorg-to", "15"], env=env, capture_output=True, text=True)
check(r.returncode == 0 and "dry run" in r.stdout and s.reorg_epoch() == 3, f"--reorg-to 15 without --yes changes nothing ({r.stdout.strip()[-80:]})")
r = subprocess.run([sys.executable, srv, "--reorg-to", "15", "--yes"], env=env, capture_output=True, text=True)
m = s.reorg_meta() or {}
check(r.returncode == 0 and m.get("epoch") == 4 and m.get("source") == "manual" and frontier() == 15,
      f"--reorg-to 15 --yes: epoch {m.get('epoch')} ({m.get('source')}), frontier {frontier()} ({(r.stdout + r.stderr).strip()[-120:]})")

shutil.rmtree(T, ignore_errors=True)
if CONTROL:
    if fails >= 1:
        print(f"CONTROL OK: {fails} check(s) fail under {CONTROL}, as they must"); sys.exit(0)
    print(f"CONTROL BROKEN: nothing failed under {CONTROL} -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
