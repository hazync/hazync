#!/usr/bin/env python3
"""REORG.md §12 "end to end" (phase P6): the whole reorg path with no person, the gate for HAZYNC_REORG_AUTO=1.

Running for real: the block fetcher (prover/fetch_block_rpc.py follow, AUTO), prep through the deployed runner
(coordinator/deploy/libzync/libzync-prep-run.sh, restarted on exit 5 as systemd does) with the host under test (its
undo records, its rewind, the record and the wait for the ack), and the coordinator (coordinator/server.py, its own
process and port) applying prep's records. Stand-ins: the node (reorg_fake_node.py serves real block files; the test
switches its active chain the way `invalidateblock` + `generatetoaddress` would), the worker (claims, beats and
submits over HTTP), and the STARK check (reorg_fake_verify.py: the synthetic branches carry no valid scripts, so no
guest can execute them; it checks a receipt's chain against prep's states as lz_coord.rs does).

Why not `bitcoind -regtest`: prep would need a regtest host, whose guest is built from source (HAZYNC_NETWORK=regtest,
the long build). The fake node is P3's (rewind_test.sh case F): mainnet 1..200 (prover/testdata) and synthetic
branches (reorg_branch.py), served through the same RPC calls the fetcher makes.

Scenarios, each on the chain the one before left: reorgs of depth 1, 2 and W, each once MID-PROVE (prep at rest at the
tip; the worker holds a claim and an unsubmitted receipt of the tip) and once MID-PREP (blocks fetched and waiting,
prep stopped between them by SIGSTOP when the node reorgs, continued once the fetcher has seen it); then depth W+1:
the fetcher and prep stop with exit 4 (the units fail; their OnFailure alert is stubbed as a log line) and nothing
moves on the board, then the runbook's one command (`libzync-prep-rewind --allow-deep`) recovers it, with the
coordinator's ack. After each: prep's tip is the node's tip (hash), the coordinator applied the record (epoch, fork,
ack), every verified range ends on prep's state for its height NOW (the board proves the node's chain), the frontier
reaches the new tip, nothing is parked, and a held orphaned receipt is refused 409 `reorged` and not counted.

  H=<host> python3 reorg_e2e.py [--w 12] [--out DIR]       # must PASS; ~5 min at W=12
  H=<host> python3 reorg_e2e.py --w 288                    # the doc's second run; longer
  H=<host> python3 reorg_e2e.py --control                  # the coordinator ignores prep's records and prep does not
                                                           # wait for an ack: the board keeps the orphaned branch;
                                                           # MUST FAIL (runs the first two scenarios only)
"""
import base64, json, os, pathlib, shutil, signal, socket, sqlite3, subprocess, sys, threading, time, urllib.error, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parents[2]
H = os.environ.get("H") or sys.exit("H=<path to the host under test>")
args = sys.argv[1:]
W = int(args[args.index("--w") + 1]) if "--w" in args else 12
CONTROL = "--control" in args
O = pathlib.Path(args[args.index("--out") + 1] if "--out" in args else f"/tmp/reorg-e2e-w{W}{'-control' if CONTROL else ''}")
J = 2 * W + 40
WAIT = float(os.environ.get("HAZYNC_TEST_E2E_WAIT_S", "900"))
MAINNET = REPO / "prover/testdata/libzync/mainnet"
GEN = "aa" * 32

shutil.rmtree(O, ignore_errors=True)
R = O / "run"                                   # LZ_ROOT
for d in (R / "bin", R / "blocks", O / "acks", O / "coord", O / "branches", O / "base"):
    d.mkdir(parents=True)
os.symlink(H, R / "bin/host_lz")
for f in (REPO / "coordinator/deploy/libzync/libzync-prep-run.sh", REPO / "prover/fetch_block_rpc.py"):
    shutil.copy(f, R / "bin" / f.name)
os.chmod(R / "bin/libzync-prep-run.sh", 0o755)
WORLD = O / "world.json"

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what, flush=True)
    fails += 0 if cond else 1
    return cond

# ---- the node --------------------------------------------------------------------------------------------------------
chain = [str(MAINNET / f"block_{h}.json") for h in range(1, 201)]
def set_world(c):
    tmp = O / ".world.tmp"
    tmp.write_text(json.dumps({"chain": c}))
    os.replace(tmp, WORLD)
_n = [0]
def branch(c, fork, length):
    """Blocks fork+1..fork+length forking from c's block `fork` (reorg_branch.py), as paths."""
    _n[0] += 1
    tag, base, out = f"e{_n[0]}", O / "base" / str(_n[0]), O / "branches" / str(_n[0])
    base.mkdir()
    for h in range(1, fork + 1):
        os.symlink(c[h - 1], base / f"block_{h}.json")
    subprocess.run([sys.executable, str(HERE / "reorg_branch.py"), str(base), str(fork), str(length), tag, str(out)],
                   check=True, stdout=subprocess.DEVNULL)
    return [str(out / f"block_{h}.json") for h in range(fork + 1, fork + length + 1)]

# ---- the common environment: the unit files' settings at test scale --------------------------------------------------
ENV = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_PREP_SAVE_EVERY="50",
           HAZYNC_PREP_UNDO_KEEP=str(J), HAZYNC_REORG_AUTO_DEPTH=str(W), HAZYNC_REORG_AUTO="1", HAZYNC_PREP_WAIT_S="3600",
           HAZYNC_FETCH_POLL_S="0.3", HAZYNC_REWIND_WAIT_S="3600", HAZYNC_REORG_ACK_DIR=str(O / "acks"),
           HAZYNC_REORG_ACK_TIMEOUT_S="300", HAZYNC_RPC_CLI=f"{sys.executable} {HERE / 'reorg_fake_node.py'} {WORLD}")
for k in [k for k in ENV if k.startswith("HAZYNC_") and k not in (
        "HAZYNC_PROFILE_LAYOUT", "HAZYNC_PREP_SAVE_EVERY", "HAZYNC_PREP_UNDO_KEEP", "HAZYNC_REORG_AUTO_DEPTH", "HAZYNC_REORG_AUTO",
        "HAZYNC_PREP_WAIT_S", "HAZYNC_FETCH_POLL_S", "HAZYNC_REWIND_WAIT_S", "HAZYNC_REORG_ACK_DIR", "HAZYNC_REORG_ACK_TIMEOUT_S",
        "HAZYNC_RPC_CLI")]:
    del ENV[k]
if CONTROL:
    ENV["HAZYNC_REORG_ACK"] = "0"

procs = []
def spawn(cmd, log, env=ENV):
    p = subprocess.Popen(cmd, stdout=open(log, "ab"), stderr=subprocess.STDOUT, env=env, start_new_session=True)
    procs.append(p)
    return p
def stop(p, sig=signal.SIGTERM):
    try: os.killpg(p.pid, sig)
    except ProcessLookupError: pass
import atexit
atexit.register(lambda: [stop(p) for p in procs])         # nothing left running when the test dies or is stopped
signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))

def start_fetcher():
    return spawn([sys.executable, str(R / "bin/fetch_block_rpc.py"), "follow", "1", str(R / "blocks"), "--window", "300",
                  "--jobs", "2", "--after-saves", str(R / "out")], O / "fetch.log")

class Runner:
    """libzync-prep-run.sh as systemd runs it: started again 1 s after an exit 5 (not in RestartPreventExitStatus),
    stopped on any other exit (exit 4 fails the unit, whose OnFailure alert is stubbed here as a line in runs.log)."""
    def __init__(self):
        self.rc, self.cur, self.exits = None, None, []
        self.t = threading.Thread(target=self.loop, daemon=True); self.t.start()
    def loop(self):
        while True:
            self.cur = spawn(["bash", str(R / "bin/libzync-prep-run.sh")], O / "run.log",
                             dict(ENV, LZ_ROOT=str(R), LZ_HI="100000"))
            rc = self.cur.wait()
            self.exits.append(rc)
            with open(O / "runs.log", "a") as f:
                f.write(f"runner exit {rc}\n" + (f"ALERT (stub): libzync-prep failed with exit {rc}\n" if rc not in (0, 5) else ""))
            if rc != 5:
                self.rc = rc; return
            time.sleep(1)
    def prep_pid(self):
        """host_lz's pid in the runner's session (never `pkill -f`: the pattern would match this test's own command)."""
        sid = self.cur.pid
        for d in pathlib.Path("/proc").iterdir():
            if d.name.isdigit():
                try:
                    if (d / "comm").read_text().strip() == "host_lz" and os.getsid(int(d.name)) == sid:
                        return int(d.name)
                except (OSError, ProcessLookupError):
                    pass
        return None

# ---- the coordinator -------------------------------------------------------------------------------------------------
PORT = (lambda s: (s.bind(("127.0.0.1", 0)), s.getsockname()[1], s.close())[1])(socket.socket())
URL = f"http://127.0.0.1:{PORT}"
CDB = O / "coord/c.db"
CENV = dict(ENV, HAZYNC_LIBZYNC="1", VERIFY_MODE="real", COORD_ALLOW_MOCK="1", HAZYNC_HOST=str(HERE / "reorg_fake_verify.py"),
            HAZYNC_BRIDGE_OUT=str(R / "leaves"), HAZYNC_LZ_STATE_DIR=str(R / "out"), HAZYNC_LZ_REORG_ACK_DIR=str(O / "acks"),
            COORD_PORT=str(PORT), COORD_BIND="127.0.0.1", COORD_DB=str(CDB), COORD_STATE=str(O / "coord/st"),
            COORD_PROOFS=str(O / "coord/proofs"), COORD_SPINE=str(O / "coord/spine"), WITNESS_DIR=str(O / "coord/w"),
            SEED_RANGES="0", TIP_HEIGHT="1", TIP_FILE=str(O / "no_tip"), GENESIS_TIP=GEN, TIP_CACHE_TTL="1",
            FRONTIER_CACHE_TTL="0.5", REORG_POLL_S="0.5", RATE_EXEMPT="127.0.0.1", PYTHONUNBUFFERED="1")
if CONTROL:
    CENV["HAZYNC_LZ_REORG_DIR"] = str(O / "nowhere")

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

def tip(h):
    try: return bytes(json.loads((R / f"out/state_{h}.json").read_text())["tip_hash"]).hex()
    except (OSError, ValueError, KeyError, TypeError): return None

def prove(h):
    """A receipt of block h on the chain prep's states say now (what the served bundle would have given the worker)."""
    for _ in range(50):
        i, o = (GEN if h == 1 else tip(h - 1)), tip(h)
        if i and o: return json.dumps({"lo": h, "hi": h, "in": i, "out": o, "n": os.urandom(6).hex()}).encode()
        time.sleep(0.1)
    return None

def submit(h, rcpt):
    return api("/api/submit", {"range": str(h), "pubkey": PK, "sig": SK.sign(rcpt).hex(), "handle": "e2e",
                               "receipt": base64.b64encode(rcpt).decode()})

def claim():
    code, out = api("/api/claim", {"pubkey": PK, "handle": "e2e"})
    rid = out.get("range") if code == 200 else None
    return int(rid) if rid and rid.isdigit() else None

def beat(h):
    ts = int(time.time())
    return api("/api/beat", {"range": str(h), "pubkey": PK, "ts": ts, "sig": SK.sign(f"{h}:{ts}".encode()).hex()})

def frontier():
    return api("/api/meta")[1].get("frontier")

def prove_to(target, hold=None):
    """Claim and prove until the frontier reaches target (or, with hold, until the claim handed out IS hold: that one
    is kept, claimed and proven, but not submitted; returned)."""
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

# ---- prep ------------------------------------------------------------------------------------------------------------
def prep_tip():
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
    check(False, f"timed out after {WAIT:.0f} s (HAZYNC_TEST_E2E_WAIT_S): {what}")
    return False

def at_node_tip():
    """prep's stores at the node's tip, on the node's block (PREP_TIP.json's hash is display hex, as the node's)."""
    h, x = prep_tip()
    return h == len(chain) and x == node_hash(h)

def board_matches_prep():
    """Every verified range ends on prep's state at its hi as it is NOW, and starts after prep's state at lo-1: the board
    holds only the node's chain. Returns the rows that do not."""
    c = sqlite3.connect(CDB)
    try: rows = c.execute("SELECT lo, hi, in_bhash, out_bhash FROM vranges").fetchall()
    finally: c.close()
    return [(lo, hi) for lo, hi, i, o in rows if o != tip(hi) or (lo > 1 and i != tip(lo - 1))]

def db1(sql, *a):
    c = sqlite3.connect(CDB)
    try: return c.execute(sql, a).fetchall()
    finally: c.close()

# ---- run -------------------------------------------------------------------------------------------------------------
print(f"== reorg end to end: W={W}, J={J}, out {O}{' (CONTROL: the board ignores prep, prep waits for no ack)' if CONTROL else ''}", flush=True)
set_world(chain[:60])
coord = spawn([sys.executable, str(REPO / "coordinator/server.py")], O / "coord.log", CENV)
fetcher = start_fetcher()
runner = Runner()
def _up():
    try: return api("/api/meta")[0] == 200
    except OSError: return False
wait(_up, "the coordinator answers")

def grow(n):
    """The node mines n blocks on its active chain (mainnet test data while it lasts, then a synthetic branch)."""
    global chain
    have = len(chain)
    if have + n <= 200 and all("testdata" in p for p in chain):
        chain = chain + [str(MAINNET / f"block_{h}.json") for h in range(have + 1, have + n + 1)]
    else:
        chain = chain + branch(chain, have, n)
    set_world(chain)
chain = chain[:60]
epoch = 0

def settle(what):
    """prep at the node's tip, the board proven to it, and the board's ranges all on prep's chain."""
    ok = wait(at_node_tip, f"{what}: prep at the node's tip {len(chain)}")
    ok = ok and check(prove_to(len(chain)) is True, f"{what}: the board proven to the tip {len(chain)} (frontier {frontier()})")
    bad = board_matches_prep()
    check(ok and not bad, f"{what}: every verified range is on prep's chain ({len(bad)} not: {bad[:4]})")
    return ok

def reorg(d, mode):
    """A reorg of depth d below prep's tip; mode 'prove' (prep at rest, the worker holding the tip) or 'prep'."""
    global chain, epoch
    tag = f"depth {d} mid-{mode}"
    print(f"== {tag}", flush=True)
    held = None
    if mode == "prove":
        # enough chain above the fork, prep at rest at the tip, the board proven to tip-1, the tip claimed and proven
        grow(3)
        wait(at_node_tip, f"{tag}: prep at the tip before the reorg")
        T = len(chain)
        prove_to(T - 1)
        held = prove_to(T, hold=T)
        check(isinstance(held, bytes), f"{tag}: the worker holds a claim and a receipt of {T} (old branch)")
        P = prep_tip()[0]
        F = P - d
        chain = chain[:F] + branch(chain, F, (len(chain) - F) + 2); set_world(chain)
        stopped_at = None
    else:
        # blocks revealed in a batch; prep stopped between them; the node reorgs below prep's tip; prep continued once the
        # fetcher has written its marker
        T0 = len(chain)
        grow(max(20, d + 10))
        wait(lambda: prep_tip()[0] >= T0 + d + 2, f"{tag}: prep under way through the new blocks")
        pid = runner.prep_pid()
        if pid: os.kill(pid, signal.SIGSTOP)
        P = prep_tip()[0]
        F = P - d
        fetched = len(chain)
        chain = chain[:F] + branch(chain, F, (len(chain) - F) + 2); set_world(chain)
        wait(lambda: (R / "blocks/REORG").exists(), f"{tag}: the fetcher's marker")
        if pid: os.kill(pid, signal.SIGCONT)
        stopped_at = (P, fetched)
    exits0 = len(runner.exits)
    epoch += 1
    ok = wait(lambda: (O / "acks" / f"{epoch}.ack").exists() or CONTROL, f"{tag}: the coordinator's ack of epoch {epoch}")
    if CONTROL:
        wait(lambda: not (R / "blocks/REORG").exists(), f"{tag}: prep rewound (control: no ack)")
    m = api("/api/meta")[1].get("reorg") or {}
    check(ok and m.get("epoch") == epoch and m.get("fork") == F,
          f"{tag}: fork {F}, prep's tip {P}{f' (stopped at {stopped_at[0]} of {stopped_at[1]} fetched)' if stopped_at else ''}; "
          f"the coordinator rewound: /api/meta reorg epoch {m.get('epoch')}, fork {m.get('fork')}")
    if held is not None:
        code, out = submit(T, held)
        att = db1("SELECT attempts FROM ranges WHERE id=?", str(T))
        # 409 `reorged`; or, while T is above what the board serves again (its bundle moved), 400 "invalid range id":
        # refused either way before anything is recorded against the block
        refused = (code == 409 and out.get("reorged")) or (code == 400 and "invalid range id" in str(out))
        check(refused and not (att and att[0][0]) and not db1("SELECT 1 FROM vranges WHERE id=?", str(T)),
              f"{tag}: the held receipt of {T} (orphaned): {code} {str(out.get('error'))[:60]}; not credited, not counted (attempts {att and att[0][0]})")
        code, out = beat(T)
        check(code in (400, 409), f"{tag}: the held claim's beat: {code} {str(out.get('error'))[:50]}")
    wait(at_node_tip, f"{tag}: prep re-prepped the new branch")
    if held is not None:
        # again, now that the board serves the new branch's block T: verification fails on the orphaned end, and a
        # reorg explains it: 409 `reorged`, not counted against the new block (REORG_FAILURE_GRACE)
        def served():
            try: return urllib.request.urlopen(f"{URL}/api/witness/{T}", timeout=10).status == 200
            except (OSError, urllib.error.HTTPError): return False
        wait(served, f"{tag}: the board serves the new branch's block {T}")
        code, out = submit(T, held)
        att = db1("SELECT attempts FROM ranges WHERE id=?", str(T))
        check(code == 409 and out.get("reorged") and not (att and att[0][0]),
              f"{tag}: the held receipt of {T} once the new branch is served: {code} reorged={out.get('reorged')}, "
              f"not counted (attempts {att and att[0][0]})")
    flog, rlog = (O / "fetch.log").read_text(errors="replace"), (O / "run.log").read_text(errors="replace")
    check(f"FETCH_REORG_AUTO prep's tip" in flog and f"fork at {F}, depth" in flog and 5 in runner.exits[exits0:]
          and f"PREP_REWOUND to {F} (epoch {epoch})" in rlog and runner.rc is None,
          f"{tag}: the fetcher saw it (auto), prep exited 5, the runner rewound to {F} with no person "
          f"(runner exits since: {runner.exits[exits0:]})")
    settle(tag)

# 0. the board on mainnet 1..60
settle("start")
if W > 100:
    grow(W + 20); settle(f"grown to {len(chain)}")
plan = [(1, "prove"), (1, "prep"), (2, "prove"), (2, "prep"), (W, "prove"), (W, "prep")]
for d, mode in (plan[:2] if CONTROL else plan):
    reorg(d, mode)
if not CONTROL:
    # W+1: beyond the automatic window: both stop with exit 4 (the units fail and alert), the board keeps what it has
    tag = f"depth {W + 1}"
    print(f"== {tag} (beyond W: a stop, then the runbook's one command)", flush=True)
    grow(W + 4)
    wait(at_node_tip, f"{tag}: prep at the tip before the reorg")
    prove_to(len(chain))
    P = prep_tip()[0]; F = P - (W + 1)
    fr0 = frontier()
    chain = chain[:F] + branch(chain, F, (len(chain) - F) + 2); set_world(chain)
    wait(lambda: fetcher.poll() is not None and runner.rc is not None, f"{tag}: the fetcher and prep stop")
    mk = json.loads((R / "blocks/REORG").read_text()) if (R / "blocks/REORG").exists() else {}
    check(fetcher.returncode == 4 and runner.rc == 4 and mk.get("class") == "manual" and mk.get("depth") == W + 1
          and "ALERT (stub)" in (O / "runs.log").read_text(),
          f"{tag}: fetcher exit {fetcher.returncode}, prep exit {runner.rc}, marker {mk.get('class')} depth {mk.get('depth')}; alert raised")
    check(frontier() == fr0 and not (O / "acks" / f"{epoch + 1}.ack").exists(), f"{tag}: the board untouched (frontier {frontier()})")
    # REORG.md §11 step 3: the one command, with both units stopped; the coordinator acks it
    rw = subprocess.run([str(R / "bin/host_lz"), "libzync-prep-rewind", str(R / "out"), str(R / "blocks"), "--allow-deep"],
                        cwd=R, env=dict(ENV, HAZYNC_COINLOG_DIR=str(R / "cl"), HAZYNC_TRIE_DIR=str(R / "trie"),
                                        HAZYNC_PREP_LEAF_BUNDLES=str(R / "leaves")), capture_output=True, text=True)
    (O / "run.log").open("a").write(rw.stdout + rw.stderr)
    epoch += 1
    m = api("/api/meta")[1].get("reorg") or {}
    check(rw.returncode == 0 and f"PREP_REWOUND to {F}" in rw.stdout and m.get("epoch") == epoch and m.get("fork") == F,
          f"{tag}: libzync-prep-rewind --allow-deep (exit {rw.returncode}), acked: epoch {m.get('epoch')}, fork {m.get('fork')}")
    fetcher = start_fetcher(); runner = Runner()             # the units started again
    settle(tag)
check(not db1("SELECT id FROM ranges WHERE status='failed'"), f"nothing parked ({db1('SELECT id FROM ranges WHERE status=?', 'failed')})")
check(len(db1("SELECT epoch FROM reorgs")) == (2 if CONTROL else 7) or CONTROL, f"reorgs applied: {len(db1('SELECT epoch FROM reorgs'))}")

for p in procs:
    stop(p)
for p in procs:
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: stop(p, signal.SIGKILL)
if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail with the board's reorg handling off, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed with the board's reorg handling off -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
