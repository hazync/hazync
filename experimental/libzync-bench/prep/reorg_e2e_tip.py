#!/usr/bin/env python3
"""REORG.md §19 (D1) and ZAPP_FOLLOWER Z2-Z5 end to end: the board follows TWO prep runs, and Coin check's tip mode reads
their tries. The P6 harness (reorg_e2e.py) with the tip run.

Running for real: two prep runs through the deployed runner (libzync-prep-run.sh, restarted on exit 5 as systemd does),
each with its own fetcher (prover/fetch_block_rpc.py) and the host under test:
  - the BOARD PASS: blocks 1..X-1 (LZ_HI = X - 1, `range`), a leaf bundle for every block, acks in acks/;
  - the TIP RUN: LZ_HI=follow (`follow`: no last block, it waits at the node's tip), HAZYNC_PREP_LEAF_FROM=X, its reorg
    records acked in acks-tip/, HAZYNC_REORG_AUTO=1;
the coordinator (coordinator/server.py, its own process) with HAZYNC_LZ_TIP_FROM=X and the tip run's directories; the
zAPP follower in tip mode (hazync-lz-zapp-follow-tip.sh) and zync-serve --tip (ZS, the binary under test) on the tip
run's disk trie. Stand-ins as reorg_e2e.py's: the node (reorg_fake_node.py: real block files, switched the way
invalidateblock + generatetoaddress would), the worker (claim, beat, submit over HTTP), verify-any
(reorg_fake_verify.py; for the follower too).

Mainnet test blocks 1..200 (prover/testdata), X = 150, W = 12, saves every 50:
  A. both runs from genesis: the board pass stops at 149 (exit 0), the tip run reaches the node's tip 170 and waits
     there; the tip run's bundles start at 150; the board proves 1..170 across X (the frontier joins the two runs'
     ranges); the two runs' states agree (no alert)
  B. the node mines 5 blocks: the tip run follows them with no restart; the board proves them
  C. the spine at 160 (the tip run's): the follower serves zync-serve at 160 from the tip run's trie, pins it; a fact
     through zync-serve at 160; at 159 (not served): 404
  D. a reorg of depth 2 at the tip, mid-prove: the tip run rewinds with no person (exit 5, the runner), its record is
     acked in acks-tip/ (none in acks/), the board pass is untouched, the held receipt is refused 409 `reorged`, the
     board proves the new branch
  E. a reorg of depth W with spine heads below and above the fork, the follower serving at the one above: the spine
     falls back, the follower serves the kept head (repins; with the older 160, still on the chain), zync-serve answers
     there, the orphaned height is no longer served; then the spine advances on the new branch and the follower serves it beside the kept head (two heights)

  H=<host> ZS=<zync-serve built with --features tip --profile zync-serve> python3 reorg_e2e_tip.py [--out DIR]   # must PASS
  H=<host> ZS=<...> python3 reorg_e2e_tip.py --control    # ONE flip: the coordinator without the tip run's variables
                                                          # (the board never serves blocks from X); MUST FAIL
"""
import base64, json, os, pathlib, shutil, signal, socket, sqlite3, subprocess, sys, threading, time, urllib.error, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
H = os.environ.get("H") or sys.exit("H=<path to the host under test>")
ZS = os.environ.get("ZS") or sys.exit("ZS=<path to zync-serve with --features tip>")
args = sys.argv[1:]
CONTROL = "--control" in args
O = pathlib.Path(args[args.index("--out") + 1] if "--out" in args else f"/tmp/reorg-e2e-tip{'-control' if CONTROL else ''}")
W, X = 12, 150
J = 2 * W + 40
WAIT = float(os.environ.get("HAZYNC_TEST_E2E_WAIT_S", "600" if not CONTROL else "180"))
MAINNET = REPO / "prover/testdata/libzync/mainnet"
GEN = "aa" * 32

shutil.rmtree(O, ignore_errors=True)
B, T_ = O / "board", O / "tip"                  # the two LZ_ROOTs
for R in (B, T_):
    for d in (R / "bin", R / "blocks"):
        d.mkdir(parents=True)
    os.symlink(H, R / "bin/host_lz")
    for f in (REPO / "coordinator/deploy/libzync/libzync-prep-run.sh", REPO / "prover/fetch_block_rpc.py"):
        shutil.copy(f, R / "bin" / f.name)
    os.chmod(R / "bin/libzync-prep-run.sh", 0o755)
for d in (O / "acks", O / "acks-tip", O / "coord", O / "branches", O / "base", O / "zserve", O / "zwork"):
    d.mkdir(parents=True)
WORLD = O / "world.json"

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what, flush=True)
    fails += 0 if cond else 1
    return cond

# ---- the node --------------------------------------------------------------------------------------------------------
chain = [str(MAINNET / f"block_{h}.json") for h in range(1, 201)]
_known = set()
def set_world(c):
    old = json.loads(WORLD.read_text())["chain"] if WORLD.exists() else []
    _known.update(p for p in old if p not in set(c))
    tmp = O / ".world.tmp"
    tmp.write_text(json.dumps({"chain": c, "known": sorted(_known)}))
    os.replace(tmp, WORLD)
_n = [0]
def branch(c, fork, length):
    _n[0] += 1
    tag, base, out = f"t{_n[0]}", O / "base" / str(_n[0]), O / "branches" / str(_n[0])
    base.mkdir()
    for h in range(1, fork + 1):
        os.symlink(c[h - 1], base / f"block_{h}.json")
    subprocess.run([sys.executable, str(HERE / "reorg_branch.py"), str(base), str(fork), str(length), tag, str(out)],
                   check=True, stdout=subprocess.DEVNULL)
    return [str(out / f"block_{h}.json") for h in range(fork + 1, fork + length + 1)]
def grow(n):
    global chain
    have = len(chain)
    if have + n <= 200 and all("testdata" in p for p in chain):
        chain = chain + [str(MAINNET / f"block_{h}.json") for h in range(have + 1, have + n + 1)]
    else:
        chain = chain + branch(chain, have, n)
    set_world(chain)

# ---- the units' settings at test scale -------------------------------------------------------------------------------
ENV = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_PREP_SAVE_EVERY="50",
           HAZYNC_PREP_UNDO_KEEP=str(J), HAZYNC_REORG_AUTO_DEPTH=str(W), HAZYNC_REORG_AUTO="1", HAZYNC_PREP_WAIT_S="3600",
           HAZYNC_FETCH_POLL_S="0.3", HAZYNC_REWIND_WAIT_S="3600", HAZYNC_REORG_ACK_TIMEOUT_S="300",
           HAZYNC_RPC_CLI=f"{sys.executable} {HERE / 'reorg_fake_node.py'} {WORLD}")
KEEP = ("HAZYNC_PROFILE_LAYOUT", "HAZYNC_PREP_SAVE_EVERY", "HAZYNC_PREP_UNDO_KEEP", "HAZYNC_REORG_AUTO_DEPTH", "HAZYNC_REORG_AUTO",
        "HAZYNC_PREP_WAIT_S", "HAZYNC_FETCH_POLL_S", "HAZYNC_REWIND_WAIT_S", "HAZYNC_REORG_ACK_TIMEOUT_S", "HAZYNC_RPC_CLI")
for k in [k for k in ENV if k.startswith("HAZYNC_") and k not in KEEP]:
    del ENV[k]
RUN_ENV = {B: dict(ENV, HAZYNC_REORG_ACK_DIR=str(O / "acks")),
           T_: dict(ENV, HAZYNC_REORG_ACK_DIR=str(O / "acks-tip"), HAZYNC_PREP_LEAF_FROM=str(X))}

procs = []
def spawn(cmd, log, env=ENV, cwd=None):
    p = subprocess.Popen(cmd, stdout=open(log, "ab"), stderr=subprocess.STDOUT, env=env, start_new_session=True, cwd=cwd)
    procs.append(p)
    return p
def stop(p, sig=signal.SIGTERM):
    try: os.killpg(p.pid, sig)
    except ProcessLookupError: pass
import atexit
atexit.register(lambda: [stop(p) for p in procs])
signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))

def start_fetcher(R):
    hi = ["range", "1", str(X - 1)] if R == B else ["follow", "1"]
    return spawn([sys.executable, str(R / "bin/fetch_block_rpc.py"), *hi, str(R / "blocks"), "--window", "300", "--jobs", "2",
                  "--after-saves", str(R / "out")], O / f"fetch-{R.name}.log", RUN_ENV[R])

class Runner:
    """libzync-prep-run.sh as systemd runs it (Restart=on-failure, RestartPreventExitStatus=3 4)."""
    def __init__(self, R, hi):
        self.R, self.hi, self.rc, self.cur, self.exits = R, hi, None, None, []
        threading.Thread(target=self.loop, daemon=True).start()
    def loop(self):
        while True:
            self.cur = spawn(["bash", str(self.R / "bin/libzync-prep-run.sh")], O / f"run-{self.R.name}.log",
                             dict(RUN_ENV[self.R], LZ_ROOT=str(self.R), LZ_HI=self.hi))
            rc = self.cur.wait()
            self.exits.append(rc)
            if rc in (0, 3, 4):
                self.rc = rc; return
            time.sleep(1)

# ---- the coordinator -------------------------------------------------------------------------------------------------
PORT = (lambda s: (s.bind(("127.0.0.1", 0)), s.getsockname()[1], s.close())[1])(socket.socket())
URL = f"http://127.0.0.1:{PORT}"
CDB = O / "coord/c.db"
CENV = dict(ENV, HAZYNC_LIBZYNC="1", VERIFY_MODE="real", COORD_ALLOW_MOCK="1", HAZYNC_HOST=str(HERE / "reorg_fake_verify.py"),
            HAZYNC_BRIDGE_OUT=str(B / "leaves"), HAZYNC_LZ_STATE_DIR=str(B / "out"), HAZYNC_LZ_REORG_ACK_DIR=str(O / "acks"),
            HAZYNC_LZ_TIP_FROM=str(X), HAZYNC_LZ_TIP_STATE_DIR=str(T_ / "out"), HAZYNC_LZ_TIP_BRIDGE_OUT=str(T_ / "leaves"),
            HAZYNC_LZ_TIP_REORG_ACK_DIR=str(O / "acks-tip"),
            COORD_PORT=str(PORT), COORD_BIND="127.0.0.1", COORD_DB=str(CDB), COORD_STATE=str(O / "coord/st"),
            COORD_PROOFS=str(O / "coord/proofs"), COORD_SPINE=str(O / "coord/spine"), WITNESS_DIR=str(O / "coord/w"),
            SEED_RANGES="0", TIP_HEIGHT="1", TIP_FILE=str(O / "no_tip"), GENESIS_TIP=GEN, TIP_CACHE_TTL="1",
            FRONTIER_CACHE_TTL="0.5", REORG_POLL_S="0.5", RATE_EXEMPT="127.0.0.1", PYTHONUNBUFFERED="1")
if CONTROL:
    for k in [k for k in CENV if k.startswith("HAZYNC_LZ_TIP_")]:
        del CENV[k]

def api(path, body=None):
    req = urllib.request.Request(URL + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")

# ---- the worker ------------------------------------------------------------------------------------------------------
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization as ser
SK = Ed25519PrivateKey.generate()
PK = SK.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw).hex()

def sdir(h):
    """The run that holds state_<h>.json, as the coordinator decides (_state_dir_for)."""
    return T_ / "out" if h >= X - 1 else B / "out"
def tip(h):
    try: return bytes(json.loads((sdir(h) / f"state_{h}.json").read_text())["tip_hash"]).hex()
    except (OSError, ValueError, KeyError, TypeError): return None
def prove(h, lo=None):
    lo = h if lo is None else lo
    for _ in range(50):
        i, o = (GEN if lo == 1 else tip(lo - 1)), tip(h)
        if i and o: return json.dumps({"lo": lo, "hi": h, "in": i, "out": o, "n": os.urandom(6).hex()}).encode()
        time.sleep(0.1)
    return None
def submit(h, rcpt, rid=None):
    return api("/api/submit", {"range": rid or str(h), "pubkey": PK, "sig": SK.sign(rcpt).hex(), "handle": "e2e",
                               "receipt": base64.b64encode(rcpt).decode()})
def claim():
    code, out = api("/api/claim", {"pubkey": PK, "handle": "e2e"})
    rid = out.get("range") if code == 200 else None
    return int(rid) if rid and rid.isdigit() else None
def frontier():
    return api("/api/meta")[1].get("frontier")
def prove_to(target, hold=None):
    t0 = time.time()
    while time.time() - t0 < WAIT:
        if hold is None and frontier() == target: return True
        h = claim()
        if h is None:
            time.sleep(0.3); continue
        r = prove(h)
        if r is None: continue
        if hold is not None and h == hold: return r
        submit(h, r)
    return None
def prep_tip(R):
    try:
        t = json.loads((R / "out/PREP_TIP.json").read_text())
        return int(t["height"]), t["hash"]
    except (OSError, ValueError, KeyError, TypeError):
        return 0, None
def node_hash(h):
    from hashlib import sha256
    import struct
    j = json.load(open(chain[h - 1]))
    hd = (struct.pack("<I", j["version"] & 0xffffffff) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
          + struct.pack("<III", j["time"], j["bits"], j["nonce"]))
    return sha256(sha256(hd).digest()).digest()[::-1].hex()
def wait(cond, what):
    t0 = time.time()
    while time.time() - t0 < WAIT:
        if cond(): return True
        time.sleep(0.2)
    check(False, f"timed out after {WAIT:.0f} s: {what}")
    return False
def tip_at_node():
    h, x = prep_tip(T_)
    return h == len(chain) and x == node_hash(h)
def db1(sql, *a):
    c = sqlite3.connect(CDB)
    try: return c.execute(sql, a).fetchall()
    finally: c.close()
def board_matches_prep():
    rows = db1("SELECT lo, hi, in_bhash, out_bhash FROM vranges")
    return [(lo, hi) for lo, hi, i, o in rows if o != tip(hi) or (lo > 1 and i != tip(lo - 1))]
def spine(h):
    r = json.dumps({"lo": 1, "hi": h, "in": GEN, "out": tip(h), "n": os.urandom(6).hex()}).encode()
    return api("/api/spine", {"pubkey": PK, "sig": SK.sign(r).hex(), "handle": "e2e", "receipt": base64.b64encode(r).decode()})
def spine_hi():
    code, out = api("/api/spine")
    return out.get("hi") if code == 200 else None

# ---- the zAPP follower in tip mode and zync-serve --tip --------------------------------------------------------------
ZPORT = (lambda s: (s.bind(("127.0.0.1", 0)), s.getsockname()[1], s.close())[1])(socket.socket())
ZURL = f"http://127.0.0.1:{ZPORT}"
(O / "zrestart").write_text(f"""#!/bin/sh
# systemctl restart hazync-lz-zync-serve-tip, at test scale (one process, its pid in a file)
[ -f {O}/zs.pid ] && kill "$(cat {O}/zs.pid)" 2>/dev/null; sleep 0.3
setsid {ZS} --tip --at-file {O}/zserve/at.json 127.0.0.1:{ZPORT} >> {O}/zync-serve.log 2>&1 &
echo $! > {O}/zs.pid
""")
(O / "zactive").write_text(f"""#!/bin/sh
kill -0 "$(cat {O}/zs.pid)" 2>/dev/null && {sys.executable} -c 'import urllib.request,sys; urllib.request.urlopen(sys.argv[1], timeout=5)' {ZURL}/tip
""")
for f in ("zrestart", "zactive"):
    os.chmod(O / f, 0o755)
atexit.register(lambda: subprocess.run(f"[ -f {O}/zs.pid ] && kill $(cat {O}/zs.pid) 2>/dev/null", shell=True))
def follow(tag):
    env = dict(ENV, ZAPP_SERVE_DIR=str(O / "zserve"), ZAPP_WORK=str(O / "zwork"), COORD_SPINE=str(O / "coord/spine"),
               ZAPP_HOST=str(HERE / "reorg_fake_verify.py"), BOARD_OUT=str(B / "out"), BOARD_TRIE=str(B / "trie"),
               TIP_FROM=str(X), TIP_OUT=str(T_ / "out"), TIP_TRIE=str(T_ / "trie"), ZAPP_PIN_W=str(W), ZAPP_OWNER="",
               ZAPP_RESTART=str(O / "zrestart"), ZAPP_ACTIVE=str(O / "zactive"), ZAPP_SETTLE_S="1", GENESIS_TIP=GEN)
    r = subprocess.run(["bash", str(REPO / "coordinator/deploy/libzync/hazync-lz-zapp-follow-tip.sh")], env=env,
                       capture_output=True, text=True, timeout=WAIT)
    (O / "follow.log").open("a").write(f"== {tag}\n" + r.stdout + r.stderr)
    return r.returncode, r.stdout + r.stderr
def served():
    try: return [a["height"] for a in json.loads((O / "zserve/at.json").read_text())]
    except (OSError, ValueError): return []
def zget(path):
    try:
        with urllib.request.urlopen(ZURL + path, timeout=10) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, {}
    except OSError:
        return None, {}
SPK = "410411db93e1dcdb8a016b49840f8c53bc1eb68a382e97b1482ecad7b148a6909a5cb2e0eaddfb84ccf9744464f82e160bfa9b8b64f9d4c03f999b8643f656b412a3ac"
def facts_at(h):
    """zync-serve answers a balance fact at h (its `at` is h) and the proof the follower published is the spine."""
    code, f = zget(f"/trie/balance/{SPK}?at={h}")
    pub = (O / "zserve/proof.rcpt").read_bytes() == (O / "coord/spine/spine.bin").read_bytes() if (O / "zserve/proof.rcpt").exists() else False
    return code == 200 and f.get("at") == h and pub

# ---- run -------------------------------------------------------------------------------------------------------------
print(f"== the tip run end to end: X={X}, W={W}, J={J}, out {O}{' (CONTROL: the coordinator without the tip run)' if CONTROL else ''}", flush=True)
chain = chain[:170]
set_world(chain)
coord = spawn([sys.executable, str(REPO / "coordinator/server.py")], O / "coord.log", CENV)
fetchers = {R: start_fetcher(R) for R in (B, T_)}
runners = {B: Runner(B, str(X - 1)), T_: Runner(T_, "follow")}
def _up():
    try: return api("/api/meta")[0] == 200
    except OSError: return False
wait(_up, "the coordinator answers")

def settle(what):
    ok = wait(tip_at_node, f"{what}: the tip run at the node's tip {len(chain)}")
    ok = ok and check(prove_to(len(chain)) is True, f"{what}: the board proven to the tip {len(chain)} across X = {X} (frontier {frontier()})")
    bad = board_matches_prep()
    check(ok and not bad, f"{what}: every verified range is on its run's chain ({len(bad)} not: {bad[:4]})")
    return ok

# A
wait(lambda: runners[B].rc is not None, "the board pass stops at X - 1")
check(runners[B].rc == 0 and prep_tip(B)[0] == X - 1, f"A: the board pass stopped at {prep_tip(B)[0]} (exit {runners[B].rc})")
ok = settle("A")
tl = sorted(int(p.name[5:-4]) for p in (T_ / "leaves").glob("leaf_*.bin"))
check(tl and min(tl) == X, f"A: the tip run's bundles start at X = {X} ({tl[:3]}..)")
m = api("/api/meta")[1]
check((m.get("tip_run") or {}).get("from") == X and not m.get("reorg_alert"), f"A: /api/meta tip_run from {X}, no alert (the runs agree): {m.get('reorg_alert')}")
if CONTROL:
    pass
else:
    # B
    ex0 = list(runners[T_].exits)
    time.sleep(3)
    check(runners[T_].cur.poll() is None, "B: the tip run waits at the node's tip (follow: no exit)")
    grow(5)
    settle("B")
    check(runners[T_].exits == ex0, f"B: the tip run followed 5 new blocks with no restart (exits {runners[T_].exits})")
    # C
    check(spine(160)[0] == 200 and spine_hi() == 160, "C: the spine at 160 (the tip run's)")
    rc, fl = follow("C")
    check(rc == 0 and served() == [160] and facts_at(160), f"C: the follower serves zync-serve --tip at 160 from the tip run's trie (exit {rc}: {fl.strip()[-160:]})")
    pin = (T_ / "trie/PIN").read_text().split()[0] if (T_ / "trie/PIN").exists() else None
    check(pin == "160" and zget(f"/trie/balance/{SPK}?at=159")[0] == 404, f"C: the tip run pinned at 160 ({pin}); 159 not served")
    # D: depth 2 at the tip, mid-prove
    grow(3)
    wait(tip_at_node, "D: the tip run at the tip")
    Tt = len(chain)
    prove_to(Tt - 1)
    held = prove_to(Tt, hold=Tt)
    check(isinstance(held, bytes), f"D: the worker holds a receipt of {Tt}")
    P = prep_tip(T_)[0]; F = P - 2
    board_out = sorted(p.name for p in (B / "out").iterdir())
    chain = chain[:F] + branch(chain, F, (len(chain) - F) + 2); set_world(chain)
    ok = wait(lambda: (O / "acks-tip/1.ack").exists(), "D: the coordinator's ack of the tip run's record 1")
    m = api("/api/meta")[1].get("reorg") or {}
    check(ok and m.get("fork") == F and m.get("source") == "prep-tip" and not list((O / "acks").iterdir()),
          f"D: the tip run's reorg (fork {F}) applied as prep-tip, acked in acks-tip/ only ({m.get('fork')}, {m.get('source')})")
    code, out = submit(Tt, held)
    check(code in (400, 409) and not db1("SELECT 1 FROM vranges WHERE id=?", str(Tt)), f"D: the held orphaned receipt of {Tt}: {code}, not credited")
    settle("D")
    check(5 in runners[T_].exits and sorted(p.name for p in (B / "out").iterdir()) == board_out,
          f"D: the tip run rewound with no person (exits {runners[T_].exits}); the board pass untouched")
    # E: depth W with the spine and the follower
    grow(3)
    settle("E: before")
    P = prep_tip(T_)[0]; F = P - W
    below, above = F - 2, len(chain) - 1
    check(spine(below)[0] == 200 and spine(above)[0] == 200 and spine_hi() == above, f"E: spine heads at {below} and {above}")
    rc, fl = follow("E: before")
    check(rc == 0 and served()[:1] == [above] and facts_at(above), f"E: the follower serves at {above} ({served()}, exit {rc}: {fl.strip()[-120:]})")
    chain = chain[:F] + branch(chain, F, (len(chain) - F) + 2); set_world(chain)
    ok = wait(lambda: (O / "acks-tip/2.ack").exists(), "E: the coordinator's ack of the tip run's record 2")
    wait(lambda: spine_hi() == below, f"E: the spine falls back to {below}")
    check(ok and spine_hi() == below, f"E: depth {W}: the spine fell back to the kept head {below} ({spine_hi()})")
    wait(tip_at_node, "E: the tip run on the new branch")
    rc, fl = follow("E: after the fall-back")
    check(rc == 0 and served()[:1] == [below] and above not in served() and facts_at(below) and zget(f"/trie/balance/{SPK}?at={above}")[0] == 404,
          f"E: the follower repins to the kept head {below}; {above} (orphaned) no longer served ({served()}, exit {rc}: {fl.strip()[-160:]})")
    settle("E")
    check(spine(len(chain))[0] == 200 and spine_hi() == len(chain), f"E: the spine advances on the new branch to {len(chain)}")
    rc, fl = follow("E: on the new branch")
    check(rc == 0 and served() == [len(chain), below] and facts_at(len(chain)) and facts_at(below),
          f"E: the follower serves {len(chain)} and the kept head {below} (two heights: {served()}, exit {rc})")
    check(not db1("SELECT id FROM ranges WHERE status='failed'"), "nothing parked")

for p in procs:
    stop(p)
if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail without the tip run on the board, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
