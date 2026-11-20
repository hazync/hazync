#!/usr/bin/env python3
"""The leaf pruner and the tip run (D1, docs/libzync/REORG.md §19): coordinator/deploy/libzync/hazync-lz-prune-leaves.sh
against a stand-in coordinator (/api/meta: frontier 50, tip_run.from 100; /api/blockstatus: 1..50 anchored, 100..104
proven, 106 folded) and a node tip of 112.

  - the board pass's bundles: at or below frontier - MARGIN (MARGIN 10: 40) go, as before; block 1 is kept
  - the tip run's: the proven 100, 101, 104 and 106 (at or below the node's tip - 6 = 106) go; 105 and 107 (not proven)
    and its sample bundle 5 (below X) stay
  - a stale node tip: no tip bundle goes, exit 2
  - without --apply: nothing goes

  python3 test_prune_leaves_tip.py              # must PASS
  python3 test_prune_leaves_tip.py --control    # the pruner without its tip-run section: the tip bundles stay; MUST FAIL
"""
import http.server, json, os, shutil, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "deploy/libzync/hazync-lz-prune-leaves.sh")
T = tempfile.mkdtemp(prefix="prune_tip_")
if "--control" in sys.argv[1:]:
    # the pruner as it was before D1: everything from the tip-run section on taken out
    old = open(SCRIPT).read().split("# the tip run's bundles (D1)")[0]
    SCRIPT = os.path.join(T, "old.sh")
    open(SCRIPT, "w").write(old)
    print("CONTROL: the pruner without its tip-run section; the checks below MUST fail")

META = {"frontier": 50, "tip_run": {"from": 100}}
STATUS = {"runs": [[1, 50, 5], [100, 104, 3], [106, 106, 4]]}


class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(META if self.path == "/api/meta" else STATUS if self.path == "/api/blockstatus" else {}).encode()
        self.send_response(200 if self.path in ("/api/meta", "/api/blockstatus") else 404)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
threading.Thread(target=srv.serve_forever, daemon=True).start()
COORD = f"http://127.0.0.1:{srv.server_address[1]}"
fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


def setup():
    for d in ("board", "tip"):
        shutil.rmtree(os.path.join(T, d), ignore_errors=True)
        os.makedirs(os.path.join(T, d))
    for h in (1, 30, 40, 41, 45):
        open(os.path.join(T, "board", f"leaf_{h}.bin"), "wb").write(b"b")
    for h in (5, 100, 101, 104, 105, 106, 107):
        open(os.path.join(T, "tip", f"leaf_{h}.bin"), "wb").write(b"t")


def left(d):
    return sorted(int(n[5:-4]) for n in os.listdir(os.path.join(T, d)) if n.startswith("leaf_"))


def run(*args, tip_age=0):
    nt = os.path.join(T, "node_tip")
    open(nt, "w").write("112\n")
    os.utime(nt, (time.time() - tip_age, time.time() - tip_age))
    env = dict(os.environ, LEAVES=os.path.join(T, "board"), COORD=COORD, MARGIN="10", TIP_LEAVES=os.path.join(T, "tip"),
               NODE_TIP_FILE=nt)
    return subprocess.run(["bash", SCRIPT, *args], env=env, capture_output=True, text=True, timeout=60)


setup()
r = run()
check(left("board") == [1, 30, 40, 41, 45] and left("tip") == [5, 100, 101, 104, 105, 106, 107],
      f"without --apply nothing goes ({r.returncode}: {r.stdout.strip()[-100:]})")
r = run("--apply")
check(left("board") == [1, 41, 45], f"the board pass's bundles at or below 40 go, block 1 kept ({left('board')})")
check(left("tip") == [5, 105, 107], f"the tip run's proven bundles at or below 106 go; 105 (open), 107 and the sample 5 stay ({left('tip')}; {r.stdout.strip()[-120:]})")
check(r.returncode == 0, f"exit 0 ({r.returncode}, {r.stderr.strip()[-100:]})")
setup()
r = run("--apply", tip_age=7200)
check(r.returncode == 2 and left("tip") == [5, 100, 101, 104, 105, 106, 107],
      f"a stale node tip: no tip bundle goes, exit 2 ({r.returncode}, {left('tip')})")
# the paper prep's cost-curve samples (HAZYNC_PREP_LEAF_AT in the shipped run.conf) are never deleted: the shipped X, every
# block proven, the node far ahead
import re
conf = open(os.path.join(HERE, "deploy/dropins/libzync-prep-run.conf")).read()
samples = [int(x) for x in re.search(r"HAZYNC_PREP_LEAF_AT=([0-9,]+)", conf).group(1).split(",")]
xs = int(re.search(r"HAZYNC_PREP_LEAF_FROM=([0-9]+)", conf).group(1))
check({500000, 600000, 700000, 800000, 900000, 962000} <= set(samples) and all(h < xs for h in samples),
      f"the shipped run.conf keeps samples at 500,000 .. 962,000, all below X = {xs}")
shutil.rmtree(os.path.join(T, "tip"), ignore_errors=True); os.makedirs(os.path.join(T, "tip"))
for h in samples + [xs, xs + 1]:
    open(os.path.join(T, "tip", f"leaf_{h}.bin"), "wb").write(b"s")
META.update(frontier=10, tip_run={"from": xs})
STATUS["runs"] = [[1, xs + 10, 3]]
open(os.path.join(T, "node_tip"), "w").write(f"{xs + 1000}\n")
env = dict(os.environ, LEAVES=os.path.join(T, "board"), COORD=COORD, MARGIN="10", TIP_LEAVES=os.path.join(T, "tip"),
           NODE_TIP_FILE=os.path.join(T, "node_tip"))
r = subprocess.run(["bash", SCRIPT, "--apply"], env=env, capture_output=True, text=True, timeout=60)
check(left("tip") == sorted(samples), f"every sample bundle survives the pruner; the proven tip bundles {xs}, {xs + 1} go ({len(left('tip'))} left; {r.stdout.strip()[-80:]})")
srv.shutdown()
shutil.rmtree(T, ignore_errors=True)
if "--control" in sys.argv[1:]:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
