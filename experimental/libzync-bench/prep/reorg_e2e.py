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

Also P5 (REORG.md §9, §10): after every settle and just before each mid-prove reorg, the offsite copy
(coordinator/deploy/hazync-offsite-proofs.py `copy --depth 6`, into an in-memory store) and its `check` run; the store
must never hold a proof of a block off the node's chain under a live name unless a marker covers it, and no marker of a
reorg no deeper than 6 below the node's tip covers anything. And the zAPP follower (hazync-lz-zapp-follow.sh, its own
stores, the real host's prep): it serves at the spine before the depth-W reorg, survives the spine dropping below its
stores (rebuilds, republishes at the kept head, the proof it serves is the spine), and follows the spine on the new
branch.

Scenarios, each on the chain the one before left (depth 2 mid-prove with the rewind dying at its `rewind-after-record`
hook and restarted as systemd would; depth W mid-prove with spine heads below and above the fork): reorgs of depth 1, 2
and W, each once MID-PROVE (prep at rest at the
tip; the worker holds a claim and an unsubmitted receipt of the tip) and once MID-PREP (blocks fetched and waiting,
prep stopped between them by SIGSTOP when the node reorgs, continued once the fetcher has seen it); then depth W+1:
the fetcher and prep stop with exit 4 (the units fail; their OnFailure alert is stubbed as a log line) and nothing
moves on the board, then the runbook's one command (`libzync-prep-rewind --allow-deep`) recovers it, with the
coordinator's ack. After each: prep's tip is the node's tip (hash), the coordinator applied the record (epoch, fork,
ack), every verified range ends on prep's state for its height NOW (the board proves the node's chain), the frontier
reaches the new tip, nothing is parked, and a held orphaned receipt is refused 409 `reorged` and not counted.

  H=<host> python3 reorg_e2e.py [--w 12] [--out DIR]       # must PASS; ~5 min at W=12
  H=<host> python3 reorg_e2e.py --w 288                    # the doc's second run; longer
  H=<host> python3 reorg_e2e.py --control                  # ONE flip: the coordinator reads no records (prep waits
                                                           # for an ack that never comes, the board keeps the orphaned
                                                           # branch); MUST FAIL (the first two scenarios only)
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
_known = set()
def set_world(c):
    # every block the node ever had stays fetchable by hash (a real node keeps stale blocks)
    old = json.loads(WORLD.read_text())["chain"] if WORLD.exists() else []
    _known.update(p for p in old if p not in set(c))
    tmp = O / ".world.tmp"
    tmp.write_text(json.dumps({"chain": c, "known": sorted(_known)}))
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
    WAIT = min(WAIT, 240.0)          # timing only: the control flips ONE thing, the coordinator's records (below)

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
    """libzync-prep-run.sh as systemd runs it (Restart=on-failure, RestartPreventExitStatus=3 4): started again 1 s after
    any exit but 0, 3 and 4 (5 = stopped for a rewind, 9 = a crash hook); stopped on those, and on 3 or 4 the unit
    fails, whose OnFailure alert is stubbed here as a line in runs.log. `once_env`: added to the next start only."""
    def __init__(self):
        self.rc, self.cur, self.exits, self.once_env = None, None, [], {}
        self.t = threading.Thread(target=self.loop, daemon=True); self.t.start()
    def loop(self):
        while True:
            extra, self.once_env = self.once_env, {}
            self.cur = spawn(["bash", str(R / "bin/libzync-prep-run.sh")], O / "run.log",
                             dict(ENV, LZ_ROOT=str(R), LZ_HI="100000", **extra))
            rc = self.cur.wait()
            self.exits.append(rc)
            with open(O / "runs.log", "a") as f:
                f.write(f"runner exit {rc}\n" + (f"ALERT (stub): libzync-prep failed with exit {rc}\n" if rc in (3, 4) else ""))
            if rc in (0, 3, 4):
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
CENV = dict(ENV, COORD_CRASH_FILE=str(O / "coord_crash"), HAZYNC_LIBZYNC="1", VERIFY_MODE="real", COORD_ALLOW_MOCK="1", HAZYNC_HOST=str(HERE / "reorg_fake_verify.py"),
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

# ---- P5: the offsite copy (an in-memory store) and the zAPP follower ---------------------------------------------------
import importlib.util, io as _io, hashlib
_spec = importlib.util.spec_from_file_location("offsite", REPO / "coordinator/deploy/hazync-offsite-proofs.py")
offmod = importlib.util.module_from_spec(_spec); _spec.loader.exec_module(offmod)
class MemS3:
    def __init__(self): self.objects = {}
    def get_paginator(self, name):
        s3 = self
        class Pg:
            def paginate(self, Bucket, Prefix):
                yield {"Contents": [{"Key": k, "Size": len(v)} for k, v in sorted(s3.objects.items()) if k.startswith(Prefix)]}
        return Pg()
    def put_object(self, Bucket, Key, Body, ContentType=None):
        assert Key not in self.objects, f"the offsite copy overwrote {Key}"
        self.objects[Key] = Body.read()
    def upload_fileobj(self, Fileobj, Bucket, Key, ExtraArgs=None, Callback=None, Config=None):
        assert Key not in self.objects, f"the offsite copy overwrote {Key}"
        self.objects[Key] = Fileobj.read()
    def get_object(self, Bucket, Key): return {"Body": _io.BytesIO(self.objects[Key])}
STORE, OFF_DEPTH, OFFP = MemS3(), 6, "proofs-46666b6f/"
(O / "keys").write_text("kid secret https://x.r2.cloudflarestorage.com b\n")
PROOFS = O / "coord/proofs"

_offchain_before = [set()]
def offchain_live(markers):
    """Live names in the store whose proof is NOT of the node's chain now (its end block is not the node's block at its
    hi) and that no marker covers."""
    out = set()
    for k, v in STORE.objects.items():
        n = k[len(OFFP):]
        if not k.startswith(OFFP) or "/" in n or not n.startswith("proof_"):
            continue
        try:
            r = json.loads(v)
            on = int(r["hi"]) <= len(chain) and r["out"] == bytes.fromhex(node_hash(int(r["hi"])))[::-1].hex()
        except (ValueError, KeyError):
            on = False
        if not on and not any(n in m.get("covered", {}) for m in markers.values()):
            out.add(n)
    return out

def offsite(tag, concurrent=False):
    """copy then check, with the node's tip as the coordinator's tip file has it; then the invariants: every live name in
    the store holds the board's proof of that name now, or a marker covers it; and against the NODE's chain: no live name
    holds a proof of a block off it unless a marker covers it. concurrent: a run while a rewind is half done (its files
    moved, not committed): no marker for it yet, and no NEW off-chain live name."""
    (O / "node_tip").write_text(f"{len(chain)}\n")
    args = ["--keys", str(O / "keys"), "--bucket", "b", "--proofs", str(PROOFS), "--method-id", "46666b6f" + "0" * 56,
            "--depth", str(OFF_DEPTH), "--tip-file", str(O / "node_tip"), "--bwlimit-mbit", "0", "--threads", "2", "--min-age=-30"]   # (a file written this second can carry an mtime a moment ahead of the clock)
    buf = _io.StringIO()
    import contextlib
    with contextlib.redirect_stdout(buf):
        rc1 = offmod.main(["copy", *args], client=STORE)
        rc2 = offmod.main(["check", *args], client=STORE)
    markers = {int(k[len(OFFP) + 7:-5]): json.loads(v) for k, v in STORE.objects.items() if k.startswith(OFFP + "reorgs/")}
    bad = []
    for k, v in STORE.objects.items():
        n = k[len(OFFP):]
        if not k.startswith(OFFP) or "/" in n:
            continue
        p = PROOFS / n
        if (not p.exists() or p.read_bytes() != v) and not any(n in m.get("covered", {}) for m in markers.values()):
            bad.append(n)
    shallow_cover = {e: len(m["covered"]) for e, m in markers.items() if e in shallow_epochs and m["covered"]}
    off = offchain_live(markers)
    if concurrent:
        check(rc1 == 0 and not (off - _offchain_before[0]),
              f"{tag}: an offsite copy while the rewind is half done: no new live name of another chain ({sorted(off - _offchain_before[0])[:3]})")
        return
    _offchain_before[0] = off
    if rc1 or rc2 or bad or shallow_cover or off:
        print("    offsite output:\n" + "\n".join("      " + l for l in buf.getvalue().splitlines()[-25:]), flush=True)
    check(rc1 == 0 and rc2 == 0 and not bad and not shallow_cover and not off,
          f"{tag}: offsite copy and check (exit {rc1}, {rc2}); no live name holds another chain's proof uncovered ({bad[:3]}), "
          f"none off the node's chain ({sorted(off)[:3]}); no marker of a reorg within {OFF_DEPTH} covers a name ({shallow_cover}); "
          f"{len(markers)} marker(s)")
shallow_epochs = set()

ZAPP = O / "zapp"; ZSERVE = O / "zserve"
(R / "bin/zapp-host").write_text(f"""#!/bin/sh
if [ "$1" = verify-any ]; then exec {sys.executable} {HERE / 'reorg_fake_verify.py'} "$@"; fi
exec {H} "$@"
""")
os.chmod(R / "bin/zapp-host", 0o755)
def follow(tag):
    """One run of the zAPP follower, as its timer runs it. Returns (exit code, log)."""
    env = dict(ENV, ZAPP_ROOT=str(ZAPP), ZAPP_SERVE_DIR=str(ZSERVE), COORD_SPINE=str(O / "coord/spine"),
               ZAPP_HOST=str(R / "bin/zapp-host"), ZAPP_FETCH=str(R / "bin/fetch_block_rpc.py"), ZAPP_OWNER="",
               ZAPP_RESTART="true", ZAPP_ACTIVE="true", ZAPP_SETTLE_S="0", GENESIS_TIP=GEN)
    for k in ("HAZYNC_REORG_AUTO", "HAZYNC_PREP_UNDO_KEEP"):
        env.pop(k, None)
    r = subprocess.run(["bash", str(REPO / "coordinator/deploy/libzync/hazync-lz-zapp-follow.sh")], env=env,
                       capture_output=True, text=True, timeout=WAIT)
    (O / "follow.log").open("a").write(f"== {tag}\n" + r.stdout + r.stderr)
    return r.returncode, r.stdout + r.stderr

def follower_serves(h):
    """The follower publishes facts at h: the proof it serves is the coordinator's spine now, its stores' block at h is
    prep's (the board's chain), and history.bin runs genesis..h."""
    try:
        rc = (ZSERVE / "proof.rcpt").read_bytes() == (O / "coord/spine/spine.bin").read_bytes()
        mine = bytes(json.loads((ZAPP / f"out/state_{h}.json").read_text())["tip_hash"]).hex()
        return rc and mine == tip(h) and (ZSERVE / "state.trie").read_bytes()[:8] == b"HZTRIE01"
    except (OSError, ValueError, KeyError):
        return False

# ---- run -------------------------------------------------------------------------------------------------------------
print(f"== reorg end to end: W={W}, J={J}, out {O}{' (CONTROL: the coordinator reads no records)' if CONTROL else ''}", flush=True)
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
    """prep at the node's tip, the board proven to it, and the board's ranges all on prep's chain; then the offsite copy."""
    ok = wait(at_node_tip, f"{what}: prep at the node's tip {len(chain)}")
    ok = ok and check(prove_to(len(chain)) is True, f"{what}: the board proven to the tip {len(chain)} (frontier {frontier()})")
    bad = board_matches_prep()
    check(ok and not bad, f"{what}: every verified range is on prep's chain ({len(bad)} not: {bad[:4]})")
    offsite(what)
    return ok

def spine(h):
    """A spine head [1..h] on prep's chain now, submitted as the worker."""
    r = json.dumps({"lo": 1, "hi": h, "in": GEN, "out": tip(h), "n": os.urandom(6).hex()}).encode()
    return api("/api/spine", {"pubkey": PK, "sig": SK.sign(r).hex(), "handle": "e2e", "receipt": base64.b64encode(r).decode()})

def spine_hi():
    code, out = api("/api/spine")
    return out.get("hi") if code == 200 else None

def reorg(d, mode, crash=None, spine_case=False, coord_crash=False):
    """A reorg of depth d below prep's tip; mode 'prove' (prep at rest, the worker holding the tip) or 'prep'. crash: a
    crash hook the rewind dies at once (HAZYNC_CRASH_AT), as systemd then restarts it. spine_case: spine heads below
    and above the fork first; the spine must fall back to the one below."""
    global chain, epoch
    tag = f"depth {d} mid-{mode}" + (f", the rewind crashing at {crash}" if crash else "") + (", with the spine" if spine_case else "")
    print(f"== {tag}", flush=True)
    held = None
    exits0 = len(runner.exits)
    if crash:
        runner.once_env = {"HAZYNC_CRASH_AT": crash}
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
        if spine_case:
            below, above = F - 2, T - 1
            check(spine(below)[0] == 200 and spine(above)[0] == 200 and spine_hi() == above,
                  f"{tag}: spine heads at {below} (below the fork) and {above} (above it)")
            rc, flog_ = follow(f"{tag}: before")
            check(rc == 0 and follower_serves(above), f"{tag}: the zAPP follower serves facts at the spine {above} (exit {rc})")
        offsite(f"{tag}: just before the reorg")
        if d <= OFF_DEPTH:
            shallow_epochs.add(epoch + 1)
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
        if coord_crash:
            (O / "coord_crash").write_text("1")      # the coordinator dies between this rewind's moves and its commit
        wait(lambda: (R / "blocks/REORG").exists(), f"{tag}: the fetcher's marker")
        if pid: os.kill(pid, signal.SIGCONT)
        stopped_at = (P, fetched)
    epoch += 1
    if coord_crash:
        global coord
        died = wait(lambda: coord.poll() is not None, f"{tag}: the coordinator dies mid-rewind (COORD_CRASH_FILE)")
        offsite(f"{tag}: the coordinator down mid-rewind", concurrent=True)
        check(died and not (O / "acks" / f"{epoch}.ack").exists() and f"{OFFP}reorgs/{epoch}.json" not in STORE.objects,
              f"{tag}: the coordinator died between the moves and the commit (exit {coord.returncode}); no ack, no offsite marker yet")
        coord = spawn([sys.executable, str(REPO / "coordinator/server.py")], O / "coord.log", CENV)     # systemd restarts it
        wait(_up, f"{tag}: the coordinator back")
    ok = wait(lambda: (O / "acks" / f"{epoch}.ack").exists(), f"{tag}: the coordinator's ack of epoch {epoch}")
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
            # its bundle is served AND the board's tip (which bounds the range ids it accepts, cached TIP_CACHE_TTL)
            # covers it: /api/block/<T> answers 404 "not mined yet" above that tip
            try:
                return (urllib.request.urlopen(f"{URL}/api/witness/{T}", timeout=10).status == 200
                        and api(f"/api/block/{T}")[0] == 200)
            except (OSError, urllib.error.HTTPError): return False
        wait(served, f"{tag}: the board serves the new branch's block {T}")
        # the range-id bound (chain_tip, exclusive) can still trail the served bundle for TIP_CACHE_TTL; /api/block/<n>
        # bounds with `n > tip` and so answers for the tip itself one block early: retry while the id is out of range
        res = {}
        def accepted_range():
            res["r"] = submit(T, held)
            return not (res["r"][0] == 400 and "invalid range id" in str(res["r"][1]))
        wait(accepted_range, f"{tag}: the board's range-id bound reaches {T}")
        code, out = res["r"]
        att = db1("SELECT attempts FROM ranges WHERE id=?", str(T))
        check(code == 409 and out.get("reorged") and not (att and att[0][0]),
              f"{tag}: the held receipt of {T} once the new branch is served: {code} reorged={out.get('reorged')}, "
              f"not counted (attempts {att and att[0][0]})")
    if crash:
        check(9 in runner.exits[exits0:] and len(db1("SELECT 1 FROM reorgs WHERE prep_epoch=?", epoch)) == 1,
              f"{tag}: the rewind died at {crash} (exits {runner.exits[exits0:]}), was finished by the restart, applied once")
    if spine_case:
        check(spine_hi() == below, f"{tag}: the spine fell back to the kept head {below} ({spine_hi()})")
        rc, flog_ = follow(f"{tag}: after the fall-back")
        check(rc == 0 and "a reorg: the spine is at" in flog_ and follower_serves(below)
              and len(list((ZAPP / "orphaned").iterdir())) == 1,
              f"{tag}: the zAPP follower survives the spine dropping below its stores: rebuilt, serving at the kept head {below} (exit {rc})")
    flog, rlog = (O / "fetch.log").read_text(errors="replace"), (O / "run.log").read_text(errors="replace")
    check(f"FETCH_REORG_AUTO prep's tip" in flog and f"fork at {F}, depth" in flog and 5 in runner.exits[exits0:]
          and f"PREP_REWOUND to {F} (epoch {epoch})" in rlog and runner.rc is None,
          f"{tag}: the fetcher saw it (auto), prep exited 5, the runner rewound to {F} with no person "
          f"(runner exits since: {runner.exits[exits0:]})")
    settle(tag)
    if mode == "prove" and d > OFF_DEPTH:
        mk = json.loads(STORE.objects.get(f"{OFFP}reorgs/{epoch}.json", b"{}"))
        rk = [k for k in STORE.objects if k.startswith(f"{OFFP}r{epoch}/")]
        check(mk.get("covered") and rk and all(STORE.objects[k] == (PROOFS / k.rsplit("/", 1)[1]).read_bytes() for k in rk),
              f"{tag}: deeper than the offsite delay: marker {epoch} covers {len(mk.get('covered') or {})} live name(s), "
              f"the new chain's proofs of them are under r{epoch}/ ({len(rk)})")
    if spine_case:
        check(spine(len(chain))[0] == 200 and spine_hi() == len(chain), f"{tag}: the spine advances on the new branch to {len(chain)}")
        rc, flog_ = follow(f"{tag}: on the new branch")
        check(rc == 0 and follower_serves(len(chain)) and "a reorg" not in flog_,
              f"{tag}: the zAPP follower follows the spine on the new branch to {len(chain)} (exit {rc})")

# 0. the board on mainnet 1..60
settle("start")
if W > 100:
    grow(W + 20); settle(f"grown to {len(chain)}")
plan = [(1, "prove", {}), (1, "prep", {}), (2, "prove", {"crash": "rewind-after-record"}), (2, "prep", {"coord_crash": True}),
        (W, "prove", {"spine_case": True}), (W, "prep", {})]
for d, mode, kw in (plan[:2] if CONTROL else plan):
    reorg(d, mode, **kw)
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
if not CONTROL:
    # review round 3, HIGH-1: prep restarts (a deploy, a crash) and replays from its save point A; the states above A
    # from before stay on disk, served; the node reorgs ABOVE prep's replaying position but below what the board has
    # verified. The fetcher sees a fork above prep's tip; prep must see that its old state files are another chain's.
    tag = "a reorg inside prep's replay window"
    print(f"== {tag} (and the zAPP follower missing the spine's fall-back)", flush=True)
    A = max(int(q.name[6:-4]) for q in (R / "out").glob("state_*.smt") if q.name[6:-4].isdigit())
    need = (A + 12) - len(chain)
    if need < 4:
        need += 50                                    # past the next save point, 12 above it
    grow(need); settle(f"{tag}: grown to {len(chain)}")
    A = max(int(q.name[6:-4]) for q in (R / "out").glob("state_*.smt") if q.name[6:-4].isdigit())
    T = len(chain)
    check(spine(T)[0] == 200, f"{tag}: the spine at {T}")
    rc, _ = follow(f"{tag}: before"); check(rc == 0 and follower_serves(T), f"{tag}: the zAPP follower serves at {T} (exit {rc})")
    runner.once_env = {"HAZYNC_PREP_LEAF_WINDOW": "1"}    # the restarted prep is held at its first bundle, mid-replay
    os.kill(runner.prep_pid(), signal.SIGKILL)
    wait(lambda: prep_tip()[0] == A and runner.prep_pid(), f"{tag}: prep restarted from its save point {A}")
    time.sleep(2)
    F = T - 3
    old = {h: tip(h) for h in range(F + 1, T + 1)}
    chain = chain[:F] + branch(chain, F, 6); set_world(chain)
    time.sleep(3)
    os.kill(runner.prep_pid(), signal.SIGKILL)        # the hold off: prep replays on into the stale states
    epoch += 1
    ok = wait(lambda: (O / "acks" / f"{epoch}.ack").exists(), f"{tag}: the coordinator's ack of epoch {epoch}")
    # the rewind prints its last line after the ack: wait for it, not just for the ack
    wait(lambda: f"PREP_REWOUND to {F}" in (O / "run.log").read_text(errors="replace"), f"{tag}: the rewind's end")
    m = api("/api/meta")[1].get("reorg") or {}
    rlog = (O / "run.log").read_text(errors="replace")
    check(ok and m.get("fork") == F and "a reorg inside the replay window" in rlog and f"PREP_REWOUND to {F}" in rlog,
          f"{tag}: prep saw its state files {F + 1}..{T} are another chain's, stopped, rewound to {F}; the board rewound (epoch {m.get('epoch')}, fork {m.get('fork')})")
    settle(tag)
    check(all(tip(h) != old[h] for h in range(F + 1, T + 1)) and frontier() == len(chain),
          f"{tag}: the board proves the new {F + 1}..{len(chain)} (frontier {frontier()})")
    # MEDIUM-1: the spine fell back (to a kept head <= F) and advances on the new branch past the follower's stores
    # before the follower runs again
    check(spine(len(chain))[0] == 200 and spine_hi() == len(chain), f"{tag}: the spine re-advanced to {len(chain)} on the new branch")
    rc, flog_ = follow(f"{tag}: after (the fall-back missed)")
    check(rc == 0 and "the node no longer has their block" in flog_ and follower_serves(len(chain)),
          f"{tag}: the zAPP follower, which never saw the fall-back, finds its block {T} gone, rebuilds and serves at {len(chain)} (exit {rc})")
    # round 4, M1: a follower run that died part-way leaves a marker, a block file of another chain and a state file
    # above its save point; the next run must not stop on them
    last = len(chain)
    (ZAPP / "blocks").mkdir(exist_ok=True)
    (ZAPP / "blocks/REORG").write_text(json.dumps({"fork": last - 2, "class": "manual", "source": "prep"}))
    shutil.copy(chain[1], ZAPP / f"blocks/block_{last + 1}.json")
    (ZAPP / f"out/state_{last + 1}.json").write_text(json.dumps({"tip_hash": list(bytes(32))}))
    grow(3); wait(at_node_tip, f"{tag}: prep at {len(chain)}"); prove_to(len(chain))
    check(spine(len(chain))[0] == 200, f"{tag}: the spine at {len(chain)}")
    rc, flog_ = follow(f"{tag}: after a run that died part-way")
    check(rc == 0 and follower_serves(len(chain)), f"{tag}: the follower clears a died run's marker, block and state files and serves at {len(chain)} (exit {rc}: {flog_.strip()[-120:]})")

check(not db1("SELECT id FROM ranges WHERE status='failed'"), f"nothing parked ({db1('SELECT id FROM ranges WHERE status=?', 'failed')})")
check(len(db1("SELECT epoch FROM reorgs")) == (2 if CONTROL else 8) or CONTROL, f"reorgs applied: {len(db1('SELECT epoch FROM reorgs'))}")

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
