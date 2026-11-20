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
srv.shutdown()
shutil.rmtree(T, ignore_errors=True)
if "--control" in sys.argv[1:]:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
