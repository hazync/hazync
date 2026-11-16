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
    shipped = "            if x: return h, x\n"
    assert src.count(shipped) == 1, "the control's patch no longer applies: update it with PrepTip.get"
    FETCH = pathlib.Path(tempfile.mkdtemp(prefix="fetch-control-")) / "fetch_block_rpc.py"
    FETCH.write_text(src.replace(shipped, "            if x: return h - 1, state_hash(self.od, h - 1)\n"))
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

def prep_states(od, node, top, save=None):
    """prep's out dir: state_<h>.json for 1..top on the node's current chain, and a complete save point at `save`."""
    od.mkdir(parents=True, exist_ok=True)
    for h in range(1, top + 1):
        (od / f"state_{h}.json").write_text(json.dumps({"height": h, "tip_hash": list(bytes.fromhex(node.active[h])[::-1])}))
    if save:
        for x in ("smt", "coinlog", "trie"): (od / f"state_{save}.{x}").write_text("x")

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

def case_below_tip(depth, W, J, n):
    """prep at the node's tip n; the top `depth` blocks are replaced. Expect a marker and exit 4."""
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, n)
        od, d = root / "out", root / "blocks"
        prep_states(od, node, n, save=n - (n % 50))
        env = {"HAZYNC_REORG_AUTO_DEPTH": str(W), "HAZYNC_PREP_UNDO_KEEP": str(J)}
        p = fetcher(root, ["follow", "1", str(d), "--after-saves", str(od), "--jobs", "2"], env)
        time.sleep(1.0)
        alive = p.poll() is None
        node.reorg(depth, "b")
        try: rc = p.wait(timeout=30 + 0.2 * depth)
        except subprocess.TimeoutExpired: p.kill(); p.wait(); rc = "timeout"
        tag = f"depth {depth} (W={W}, J={J})"
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
        prep_states(od, node, n - 6, save=50)
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
            prep_states(od, node, n)
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
        prep_states(od, node, 90, save=50)                                # saved at 50, stale states up to 90
        p = fetcher(root, ["range", "1", "100", str(d), "--after-saves", str(od), "--jobs", "2"], {})
        try:
            ok = wait_until(lambda: all((d / f"block_{h}.json").exists() for h in range(51, 101)), 60)
            check(ok and p.poll() is None and not (d / "REORG").exists(), "stale states above the save point: 51..100 fetched, no marker")
        finally:
            p.kill(); p.wait()
    finally:
        shutil.rmtree(root, ignore_errors=True)

def case_fork_point_deep():
    root = pathlib.Path(tempfile.mkdtemp(prefix="fetch-reorg-"))
    try:
        (root / "fakecli.py").write_text(FAKE)
        node = Node(root, 120)
        od = root / "out"
        prep_states(od, node, 120)
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
