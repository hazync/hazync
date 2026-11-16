#!/usr/bin/env python3
"""fetch_block_rpc.py's reorg detection (docs/libzync/REORG.md §4, phase P2), against a FAKE bitcoin-cli.

The fake node is a JSON file of blocks (real header hashes: the fetcher checks a waiting file by hashing its header) and
the active chain by height; a reorg rewrites the active chain from the fork up. Prep is simulated by its out dir: one
state_<h>.json per height with the block's tip_hash (the only field the fetcher reads). The fetcher runs as the units
run it, a subprocess with --after-saves.

  python3 prover/test_fetch_reorg.py            # must PASS
  python3 prover/test_fetch_reorg.py --control  # detection compares a height prep is not at: MUST FAIL

The control runs the cases below prep's tip against a COPY of the fetcher whose comparison is pointed one block below
prep's tip (the shipped file is not changed). A 1-block reorg of prep's tip must then go unseen: the control passes
only if the depth-1 case fails by timing out with no marker.

Small W and J (W=12, J=40) keep it fast; the depths W, W+1 and J+1 are also run once at the shipped W=288, J=2016.
"""
import hashlib, json, os, pathlib, shutil, struct, subprocess, sys, tempfile, time

HERE = pathlib.Path(__file__).resolve().parent
FETCH = HERE / "fetch_block_rpc.py"
CONTROL = "--control" in sys.argv
if CONTROL:
    src = FETCH.read_text()
    # both ways PrepTip.get answers: from PREP_TIP.json, and from the state files (a host from before it)
    patches = [("                return h, x\n", "                return h - 1, state_hash(self.od, h - 1)\n"),
               ("            if x: return h, x\n", "            if x: return h - 1, state_hash(self.od, h - 1)\n")]
    for a, b in patches:
        assert src.count(a) == 1, "the control's patch no longer applies: update it with PrepTip.get"
        src = src.replace(a, b)
    FETCH = pathlib.Path(tempfile.mkdtemp(prefix="fetch-control-")) / "fetch_block_rpc.py"
    FETCH.write_text(src)
FAKE = r'''
import json, sys
w = json.load(open(sys.argv[1])); m, a = sys.argv[2], sys.argv[3:]
act, blk = w["active"], w["blocks"]
def out(x): print(x if isinstance(x, str) else json.dumps(x)); sys.exit(0)
def die(x): print(x, file=sys.stderr); sys.exit(1)
if m == "getblockchaininfo": out({"blocks": len(act) - 1, "bestblockhash": act[-1]})
if m == "getbestblockhash": out(act[-1])
if m == "getblockcount": out(len(act) - 1)
if m == "getblockhash":
    h = int(a[0]); out(act[h]) if 0 <= h < len(act) else die("error code: -8\nBlock height out of range")
b = blk.get(a[0]) or die("error code: -5\nBlock not found")
hdr = {"hash": a[0], "height": b["height"], "version": b["version"], "time": b["time"], "mediantime": b["time"],
       "bits": "%08x" % b["bits"], "nonce": b["nonce"], "merkleroot": b["merkle"]}
if b["prev"] != "00" * 32: hdr["previousblockhash"] = b["prev"]
if m == "getblockheader": out(hdr)
if m == "getblock": out(dict(hdr, tx=[{"hex": "00"}]) if a[1:] == ["3"] else dict(hdr, tx=["00" * 32]))
die("fake: " + m)
'''

def header_hash(b):
    hdr = (struct.pack("<I", b["version"]) + bytes.fromhex(b["prev"])[::-1] + bytes.fromhex(b["merkle"])[::-1]
           + struct.pack("<III", b["time"], b["bits"], b["nonce"]))
    return hashlib.sha256(hashlib.sha256(hdr).digest()).digest()[::-1].hex()

class Node:
    def __init__(self, root, n):
        self.path = root / "world.json"
        self.blocks, self.active = {}, []
        self.extend("a", n + 1)
    def mk(self, branch, h, prev):
        b = {"height": h, "version": 0x20000000, "prev": prev, "time": 1_600_000_000 + 600 * h, "bits": 0x207fffff,
             "nonce": 0, "merkle": hashlib.sha256(f"{branch}-{h}".encode()).hexdigest()}
        x = header_hash(b)
        self.blocks[x] = b
        return x
    def extend(self, branch, count):
        for _ in range(count):
            h = len(self.active)
            self.active.append(self.mk(branch, h, self.active[-1] if h else "00" * 32))
        self.save()
    def reorg(self, depth, branch, extra=1):
        """The top `depth` blocks replaced by `depth + extra` blocks of another branch."""
        del self.active[len(self.active) - depth:]
        self.extend(branch, depth + extra)
    def save(self):
        tmp = self.path.with_suffix(".tmp")
        tmp.write_text(json.dumps({"blocks": self.blocks, "active": self.active}))
        os.replace(tmp, self.path)

def prep_states(od, node, top, save=None, tip="top", keep=None, undo_from=1):
    """prep's out dir: state_<h>.json for 1..top on the node's current chain, a complete save point at `save`, and
    PREP_TIP.json (prep's real position, as reorg.rs write_tip writes it) at `tip` ("top" = top; None = no tip file, a
    host from before it). `keep` = J in the tip file (default: the fetcher's env J)."""
    od.mkdir(parents=True, exist_ok=True)
    for h in range(1, top + 1):
        (od / f"state_{h}.json").write_text(json.dumps({"height": h, "tip_hash": list(bytes.fromhex(node.active[h])[::-1])}))
    if save:
        for x in ("smt", "coinlog", "trie"): (od / f"state_{save}.{x}").write_text("x")
    write_tip(od, node, top if tip == "top" else tip, keep, undo_from)

def write_tip(od, node, h, keep=None, undo_from=1):
    if h is None: (od / "PREP_TIP.json").unlink(missing_ok=True); return
    k = keep if keep is not None else int(os.environ.get("TEST_TIP_KEEP", "2016"))
    (od / "PREP_TIP.json.tmp").write_text(json.dumps({"height": h, "hash": node.active[h], "undo_keep": k, "undo_from": undo_from}))
    os.replace(od / "PREP_TIP.json.tmp", od / "PREP_TIP.json")

fails, ran = [], []
def check(ok, what):
    ran.append(what)
    print(f"  {'ok  ' if ok else 'FAIL'} {what}", flush=True)
    if not ok: fails.append(what)

def fetcher(root, args, env_extra):
    env = dict(os.environ, HAZYNC_RPC_CLI=f"{sys.executable} {root / 'fakecli.py'} {root / 'world.json'}",
               HAZYNC_FETCH_POLL_S="0.2", **env_extra)
    log = open(root / "fetch.log", "a")
    return subprocess.Popen([sys.executable, str(FETCH)] + args, env=env, stdout=log, stderr=subprocess.STDOUT)

def wait_until(cond, t=60):
    end = time.time() + t
    while time.time() < end:
        if cond(): return True
        time.sleep(0.1)
    return False

def files_hashes(d):
    sys.path.insert(0, str(HERE))
    import importlib.util
    spec = importlib.util.spec_from_file_location("fbr", FETCH)
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    return {int(q.name[6:-5]): m.file_block(q)[1] for q in d.glob("block_*.json")}

def case_below_tip(depth, W, J, n, tipfile=True):
    """prep at the node's tip n; the top `depth` blocks are replaced. Expect a marker and exit 4. tipfile=False: prep
    from before PREP_TIP.json (the fetcher finds its tip from the state files)."""
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, n)
        od, d = root / "out", root / "blocks"
        prep_states(od, node, n, save=n - (n % 50), keep=J, tip=n if tipfile else None)
        env = {"HAZYNC_REORG_AUTO_DEPTH": str(W), "HAZYNC_PREP_UNDO_KEEP": str(J)}
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od), "--jobs", "2"], env)
        time.sleep(1.0)
        alive = p.poll() is None
        node.reorg(depth, "b")
        try: rc = p.wait(timeout=30 + 0.2 * depth)
        except subprocess.TimeoutExpired: p.kill(); p.wait(); rc = "timeout"
        tag = f"depth {depth} (W={W}, J={J}{'' if tipfile else ', no tip file'})"
        if depth > J:
            m = d / "REORG_TOO_DEEP"
            body = json.loads(m.read_text()) if m.exists() else {}
            check(alive and rc == 4 and body.get("prep_tip") == n and body.get("deepest_compared") == n - J and not (d / "REORG").exists(),
                  f"{tag}: REORG_TOO_DEEP, exit 4 (rc={rc}, {body.get('deepest_compared')})")
        else:
            m = d / "REORG"
            body = json.loads(m.read_text()) if m.exists() else {}
            want_cls = "auto" if depth <= W else "manual"
            ok = (alive and rc == 4 and body.get("fork") == n - depth and body.get("depth") == depth
                  and body.get("class") == want_cls and len(body.get("orphaned", [])) == depth
                  and [x[1] for x in body.get("new", [])] == node.active[n - depth + 1:n + 1]
                  and all(x[1] != y for x, y in zip(body.get("orphaned", []), node.active[n - depth + 1:])))
            check(ok, f"{tag}: REORG fork {body.get('fork')} depth {body.get('depth')} class {body.get('class')} (want {want_cls}), exit 4 (rc={rc})")
        # a second start refuses while the marker is there
        p2 = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], env)
        try: rc2 = p2.wait(timeout=60)
        except subprocess.TimeoutExpired: p2.kill(); p2.wait(); rc2 = "timeout"
        check(rc2 == 4, f"{tag}: a restart with the marker there exits 4 at once (rc={rc2})")
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_above_tip():
    """prep 6 below the node's tip; a 3-block reorg above prep's tip: the orphaned waiting files are replaced, no stop."""
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        n = 60
        node = Node(root, n)
        od, d = root / "out", root / "blocks"
        prep_states(od, node, n - 6, save=50, keep=40)
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od), "--jobs", "3"], {"HAZYNC_REORG_AUTO_DEPTH": "12", "HAZYNC_PREP_UNDO_KEEP": "40"})
        try:
            got = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(n - 5, n + 1)))
            before = files_hashes(d)
            node.reorg(3, "c", extra=2)                                   # fork at n-3, new tip n+2
            want = {h: node.active[h] for h in range(51, n + 3)}          # from prep's save point 50 to the new tip
            same = wait_until(lambda: files_hashes(d) == want, 60)
            check(got and same and p.poll() is None and not (d / "REORG").exists(),
                  f"fork above prep's tip: waiting files {n - 2}..{n} replaced, {n + 1}..{n + 2} fetched, no marker, still running")
            check(before[n - 4] == files_hashes(d)[n - 4], "a waiting file below the fork is kept, not fetched again")
            # prep then reaches the old tip on the NEW chain: still nothing to stop for
            prep_states(od, node, n, keep=40)
            time.sleep(1.5)
            check(p.poll() is None and not (d / "REORG").exists(), "prep on the new chain: no marker")
        finally:
            p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_node_behind_and_range_done():
    """The node below prep's tip on prep's chain (a node still syncing): no marker, no exit. Then range mode exits 0 once
    prep reaches hi."""
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, 40)
        od, d = root / "out", root / "blocks"
        prep_states(od, node, 40)
        full = list(node.active)
        node.active = full[:31]; node.save()                             # the node "resyncing", at 30
        p = fetcher(root, ["range", "1", "45", str(d), "--after-saves", str(od)], {})
        time.sleep(2.0)
        check(p.poll() is None and not (d / "REORG").exists() and not list(d.glob("block_*.json")),
              "node behind prep on its chain: waits, fetches nothing, no marker")
        node.active = full; node.extend("a", 5)                           # caught up and moved on to 45
        ok = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(41, 46)), 60)
        prep_states(od, node, 45, save=45)
        rc = p.wait(timeout=60)
        check(ok and rc == 0, f"range: fetched 41..45, exited 0 once prep reached hi (rc={rc})")
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_stale_states():
    """State files above prep's newest save point left by an older run (the same chain): fetching still starts right
    after the save point, where prep resumes, not after the stale files."""
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, 100)
        od, d = root / "out", root / "blocks"
        for tipf in (50, None):                                          # prep restarted from 50; and an old host
            shutil.rmtree(od, ignore_errors=True); shutil.rmtree(d, ignore_errors=True)
            prep_states(od, node, 90, save=50, tip=tipf)                  # saved at 50, stale states up to 90
            p = fetcher(root, ["range", "1", "100", str(d), "--after-saves", str(od), "--jobs", "2"], {})
            try:
                ok = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(51, 101)), 60)
                check(ok and p.poll() is None and not (d / "REORG").exists(),
                      f"stale states above the save point ({'tip file at 50' if tipf else 'no tip file'}): 51..100 fetched, no marker")
            finally:
                p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)

def run_once(root, od, d, env=None, timeout=30):
    """One fetcher run that should stop at once: (exit code or 'timeout', marker name or None, marker body)."""
    p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], dict({"HAZYNC_REORG_AUTO_DEPTH": "12", "HAZYNC_PREP_UNDO_KEEP": "40"}, **(env or {})))
    try: rc = p.wait(timeout=timeout)
    except subprocess.TimeoutExpired: p.kill(); p.wait(); rc = "timeout"
    for m in ("REORG", "REORG_TOO_DEEP"):
        if (d / m).exists(): return rc, m, json.loads((d / m).read_text())
    return rc, None, {}

def setup(n):
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    (root / "fakecli.py").write_text(FAKE)
    return root, Node(root, n), root / "out", root / "blocks"

def case_review_findings():
    """The 16.11.2026 review's cases, one each (M1, M2, M4, L1, L2)."""
    # M1: no tip file (an old host); the three newest state files torn, the fork BELOW them: still a stop
    root, node, od, d = setup(120)
    try:
        prep_states(od, node, 110, save=100, tip=None)
        for h in (108, 109, 110): (od / f"state_{h}.json").write_text("")
        node.active = node.active[:96]; node.extend("x", 25)              # fork at 95
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m == "REORG" and b.get("fork") == 95 and b.get("prep_tip") == 107,
              f"M1: torn newest states, fork at 95: REORG fork {b.get('fork')} from prep's tip {b.get('prep_tip')} (rc={rc})")
        shutil.rmtree(d)
        for h in range(100, 111): (od / f"state_{h}.json").write_text("")
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m is None and "FETCH_STOPPED_CANNOT_CHECK" in (root / "fetch.log").read_text(),
              f"M1: no state down to the save point reads: exit 4 'cannot check', never 'no reorg' (rc={rc}, marker {m})")
    finally:
        shutil.rmtree(root, ignore_errors=True)
    # M2: prep restarted from 100 and at 102 (its tip file); stale states to 110 from the crashed run; the node forks
    # at 105, ABOVE prep's real tip: no stop, and the waiting files above 102 are the node's
    root, node, od, d = setup(120)
    try:
        prep_states(od, node, 110, save=100, tip=102)
        node.active = node.active[:106]; node.extend("y", 20)             # fork at 105
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], {})
        try:
            ok = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(103, 126)), 60)
            fh = files_hashes(d)
            check(ok and p.poll() is None and not (d / "REORG").exists() and all(fh[h] == node.active[h] for h in range(103, 126)),
                  "M2: a fork above prep's real tip (102; stale states to 110): no marker, files 103..125 on the node's chain")
        finally:
            p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)
    # M4: a state file missing inside the walk: 'cannot check', not a deeper fork
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40)
        (od / "state_55.json").unlink()
        node.reorg(10, "z")                                               # fork at 50
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m is None and "state_55.json is missing" in (root / "fetch.log").read_text(),
              f"M4: a missing state inside the walk: exit 4 'cannot check', no marker with a wrong fork (rc={rc}, marker {m})")
    finally:
        shutil.rmtree(root, ignore_errors=True)
    # L1: the node BEHIND prep and on another branch: depth counted from prep's tip 60 (fork 15: 45 > J=40)
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40)
        node.active = node.active[:16]; node.extend("w", 10)              # node at 25, forked at 15
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m == "REORG_TOO_DEEP", f"L1: node behind prep, fork 45 below prep's tip (J=40): REORG_TOO_DEEP, not manual (rc={rc}, {m})")
    finally:
        shutil.rmtree(root, ignore_errors=True)
    # L2: J and the records' reach from prep's tip file: records only from 55 (the host was upgraded at 54), fork at 50
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40, undo_from=55)
        node.reorg(10, "v")
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m == "REORG_TOO_DEEP" and b.get("fork") == 50 and b.get("undo_from") == 55,
              f"L2: a fork (50) below the lowest undo record (55): REORG_TOO_DEEP (rc={rc}, {m}, {b.get('why')})")
        shutil.rmtree(d)
        # the same fork with records from 45: reached, depth 10 <= W=12: REORG auto
        t = json.loads((od / "PREP_TIP.json").read_text()); t["undo_from"] = 45
        (od / "PREP_TIP.json").write_text(json.dumps(t))
        rc, m, b = run_once(root, od, d)
        check(rc == 4 and m == "REORG" and b.get("fork") == 50 and b.get("class") == "auto",
              f"L2: the same fork with records from 45: REORG auto (rc={rc}, {m})")
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_new_file_same_best():
    """L4: a waiting file from another branch appears while the node's best block is unchanged (the node went to that
    branch and back between two polls): it is checked as a NEW file at the next poll, deleted and fetched again."""
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 50, save=50, keep=40)
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], {})
        try:
            ok = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(51, 61)), 60)
            time.sleep(1.0)
            other = Node.__new__(Node); other.blocks = {}
            hb = other.mk("u", 58, node.active[57])
            b = other.blocks[hb]
            (d / "block_58.json.x").write_text(json.dumps({"height": 58, "version": b["version"], "time": b["time"], "bits": b["bits"],
                                                            "nonce": b["nonce"], "prev": b["prev"], "merkle": b["merkle"], "txs": []}))
            os.replace(d / "block_58.json.x", d / "block_58.json")
            back = wait_until(lambda: files_hashes(d).get(58) == node.active[58], 30)
            check(ok and back and p.poll() is None and not (d / "REORG").exists(),
                  "L4: a file from another branch with the best block unchanged: replaced by the node's block 58")
        finally:
            p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_auto_rewind():
    """P3, HAZYNC_REORG_AUTO=1: a fork within W writes REORG (class auto) and the fetcher WAITS (no exit) while prep
    rewinds; when the marker goes (prep's rewind: save point and tip at the fork, the states above moved), it fetches
    the new branch from the fork. A fork beyond W still stops (exit 4)."""
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40)
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], {"HAZYNC_REORG_AUTO": "1", "HAZYNC_REORG_AUTO_DEPTH": "12"})
        try:
            time.sleep(2.0)
            node.reorg(3, "r")                                             # fork at 57, new tip 61
            marked = wait_until(lambda: (d / "REORG").exists(), 30)
            body = json.loads((d / "REORG").read_text()) if marked else {}
            time.sleep(1.5)
            waiting = p.poll() is None and "waiting for prep's rewind" in (root / "fetch.log").read_text()
            # prep's rewind, as libzync-prep-rewind leaves it
            for h in (58, 59, 60): (od / f"state_{h}.json").unlink()
            for x in ("smt", "coinlog", "trie"): (od / f"state_57.{x}").write_text("x")
            write_tip(od, node, 57, keep=40)
            (d / "REORG").unlink()
            fetched = wait_until(lambda: all(files_hashes(d).get(h) == node.active[h] for h in range(58, 62)), 60)
            check(marked and body.get("class") == "auto" and body.get("fork") == 57 and waiting and fetched and p.poll() is None,
                  f"AUTO: a fork within W: REORG auto, the fetcher waits (no exit), then fetches 58..61 of the new branch (rc={p.poll()})")
        finally:
            p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40)
        node.reorg(13, "s")
        rc, m, b = run_once(root, od, d, {"HAZYNC_REORG_AUTO": "1"})
        check(rc == 4 and m == "REORG" and b.get("class") == "manual", f"AUTO: a fork beyond W (13) still stops: exit 4, class manual (rc={rc})")
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_node_reindexing():
    """Review 16.11 (low): a node reindexing from scratch (at genesis) is a node behind prep: wait, no REORG_TOO_DEEP."""
    root, node, od, d = setup(60)
    try:
        prep_states(od, node, 60, save=50, keep=40)
        full = list(node.active); node.active = full[:1]; node.save()
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od)], {})
        time.sleep(2.0)
        check(p.poll() is None and not (d / "REORG_TOO_DEEP").exists() and not (d / "REORG").exists(),
              "a node at genesis (reindexing): waits, no marker")
        p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_fork_point_deep():
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, 120)
        od = root / "out"
        prep_states(od, node, 120, keep=40)
        node.reorg(70, "d")
        env = dict(os.environ, HAZYNC_RPC_CLI=f"{sys.executable} {root / 'fakecli.py'} {root / 'world.json'}",
                   HAZYNC_PREP_UNDO_KEEP="40", HAZYNC_REORG_AUTO_DEPTH="12")
        run = lambda *a: json.loads(subprocess.run([sys.executable, str(FETCH), "fork-point", str(od), *a], env=env,
                                                   capture_output=True, text=True, timeout=120).stdout)
        a, b = run(), run("--deep")
        check(a["fork"] is None and a["deepest_compared"] == 80 and b["fork"] == 50 and b["depth"] == 70
              and not b["within_J"] and not b["within_W"], f"fork-point: none within J, --deep finds 50 depth 70 ({a}, {b})")
    finally:
        shutil.rmtree(root, ignore_errors=True)

if __name__ == "__main__":
    print("== forks below prep's tip (W=12, J=40)")
    for depth in (1, 2, 12, 13, 41):
        case_below_tip(depth, 12, 40, 60)
    if not CONTROL:
        print("== forks above prep's tip, a node behind, range mode, fork-point")
        case_above_tip()
        case_node_behind_and_range_done()
        case_stale_states()
        case_fork_point_deep()
        print("== the review's cases (16.11.2026)")
        case_review_findings()
        case_new_file_same_best()
        case_node_reindexing()
        print("== P3: automatic rewinds")
        case_auto_rewind()
        print("== an old host without PREP_TIP.json")
        for depth in (1, 13, 41):
            case_below_tip(depth, 12, 40, 60, tipfile=False)
        print("== the shipped window (W=288, J=2016)")
        for depth in (288, 289, 2017):
            case_below_tip(depth, 288, 2016, 2030)
    print()
    if CONTROL:
        shutil.rmtree(FETCH.parent, ignore_errors=True)
        unseen = [f for f in fails if f.startswith("depth 1 (W=12, J=40): REORG fork None") and "rc=timeout" in f]
        if unseen:
            print(f"CONTROL OK: with the comparison one block below prep's tip, the 1-block reorg went unseen ({len(fails)} failure(s))")
            sys.exit(0)
        print(f"CONTROL FAILED: the depth-1 reorg was still caught; failures: {fails}")
        sys.exit(1)
    if fails:
        print(f"FAILED {len(fails)} of {len(ran)}: " + "; ".join(fails)); sys.exit(1)
    print(f"all {len(ran)} checks passed: every fork below prep's tip stops with a marker and exit 4, one above it is repaired")
