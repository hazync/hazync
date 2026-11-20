#!/usr/bin/env python3
"""The zAPP follower in tip mode (coordinator/deploy/libzync/hazync-lz-zapp-follow-tip.sh; docs/libzync/ZAPP_FOLLOWER.md
§5-§7, Z3-Z5), with stand-ins: the host (`verify-any`: a spine file names its height and block; --check-end compares
it with the state in HAZYNC_LZ_STATE_DIR, as lz_coord.rs does), the two prep runs (state files, roots.log, INTACT,
PREP_TIP.json) and zync-serve (its restart checks every height of at.json the way `zync-serve --tip` opens it: logged
in its trie_dir's roots.log with that block and trie_root, and not released; up or down).

The board pass holds roots from 40 (its Z1 restart), the tip run (X = 100) from 90. W = 12.
  1. no spine: nothing to do
  2. the spine at 30: below the board pass's first root: not served; the board pass is PINNED at 30 (the bootstrap)
  3. at 50: served at [50] from the board pass, the proof published, the pin at 50
  4. at 55: served at [55, 50] (two heights, Z4), the pin at 50
  5. at 99 = X - 1: the source is the tip run: [99 (tip run), 55 (board pass)]; both pinned
  6. at 110: [110, 99], both the tip run's; the board pass UNPINNED; the tip run pinned at the fall-back head 99
     (the newest kept head at or below its position 120 - W)
  7. a reorg (Z5): the tip run rewinds to 105, the spine falls back to the kept head 99: served at [99] (110 dropped:
     not the tip run's block any more)
  8. a fall-back to 99 after 99 was released (INTACT 100): the run fails (alerts), the served set is unchanged
  9. a spine that is not the source's chain: the run fails, nothing changes

  python3 test_zapp_follow_tip.py            # must PASS
  python3 test_zapp_follow_tip.py --control  # the follower never switches to the tip run: MUST FAIL
"""
import hashlib, json, os, shutil, stat, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "deploy/libzync/hazync-lz-zapp-follow-tip.sh")
CONTROL = "--control" in sys.argv[1:]
T = tempfile.mkdtemp(prefix="zapp_tip_")
if CONTROL:
    src = open(SCRIPT).read()
    old = 'if [ -n "$TIP_FROM" ] && [ "$1" -ge $((TIP_FROM - 1)) ]; then'
    assert src.count(old) == 1
    SCRIPT = os.path.join(T, "follow.sh")
    open(SCRIPT, "w").write(src.replace(old, "if false; then"))
    print("CONTROL: the follower never switches to the tip run; the checks below MUST fail")
X, W = 100, 12
D = {k: os.path.join(T, k) for k in ("board_out", "board_trie", "tip_out", "tip_trie", "serve", "spine", "work")}
for d in D.values():
    os.makedirs(d)
os.makedirs(os.path.join(D["spine"], "heads"))
HOST = os.path.join(T, "host")
open(HOST, "w").write(r'''#!/usr/bin/env python3
import json, os, sys
a = sys.argv[1:]
s = json.load(open(a[1]))
end = ""
if "--check-end" in a:
    try: t = bytes(json.load(open(os.path.join(os.environ["HAZYNC_LZ_STATE_DIR"], f"state_{s['hi']}.json")))["tip_hash"]).hex()
    except Exception: t = None
    if t != s["bh"]:
        print(f"the receipt ends on a state at block {s['hi']} that is not the board's", file=sys.stderr); sys.exit(1)
    end = " end=board"
print(f"RANGE-OK lo=1 hi={s['hi']} in_tip=g out_tip=o out_leaves=1 range_work=0 in_bhash=00 out_bhash={s['bh']} image=x lineage=y anchored=1{end}")
''')
ZS = os.path.join(T, "zync-serve-restart")
STATUS = os.path.join(T, "status")
open(ZS, "w").write(r'''#!/usr/bin/env python3
# zync-serve --tip's start checks, on the follower's at.json: each height logged with its block and trie_root, not released
import json, os, struct, sys
ok = True
for a in json.load(open(sys.argv[1])):
    b = open(os.path.join(a["trie_dir"], "roots.log"), "rb").read()
    e = [b[80 * i:80 * i + 80] for i in range(len(b) // 80)]
    hit = [x for x in e if struct.unpack_from("<I", x)[0] == a["height"] and x[4:36].hex() == a["hash"]]
    try: intact = int(open(os.path.join(a["trie_dir"], "INTACT")).read().split()[1])
    except Exception: intact = 0
    ok = ok and bool(hit) and hit[-1][44:76].hex() == a["trie_root"] and a["height"] >= intact
open(sys.argv[2], "w").write("up" if ok else "down")
''')
for f in (HOST, ZS):
    os.chmod(f, os.stat(f).st_mode | stat.S_IEXEC)

def bh(tag, h): return hashlib.sha256(f"{tag}{h}".encode()).hexdigest()
def root(tag, h): return hashlib.sha256(f"root{tag}{h}".encode()).hexdigest()
def chain(tag, h): return "B" if tag == "B" and h > 105 else "A"
def prep(run, lo, hi, tag="A", roots_from=None):
    out, trie = D[f"{run}_out"], D[f"{run}_trie"]
    with open(os.path.join(trie, "roots.log"), "ab") as f:
        for h in range(lo, hi + 1):
            t = chain(tag, h)
            json.dump({"height": h, "tip_hash": list(bytes.fromhex(bh(t, h))), "trie_root": list(bytes.fromhex(root(t, h)))},
                      open(os.path.join(out, f"state_{h}.json"), "w"))
            if roots_from is None or h >= roots_from:
                f.write(struct.pack("<I", h) + bytes.fromhex(bh(t, h)) + struct.pack("<Q", h * 10) + bytes.fromhex(root(t, h)) + struct.pack("<I", 1))
    json.dump({"height": hi}, open(os.path.join(out, "PREP_TIP.json"), "w"))
def intact(run, h): open(os.path.join(D[f"{run}_trie"], "INTACT"), "w").write(f"intact_from {h} gen 1 flushed {h}\n")
def spine(h, tag="A", keep=True):
    s = json.dumps({"hi": h, "bh": bh(chain(tag, h), h)})
    open(os.path.join(D["spine"], "spine.bin"), "w").write(s)
    if keep:
        open(os.path.join(D["spine"], "heads", f"spine_{h}.json"), "w").write(s)
def pin(run):
    try: return int(open(os.path.join(D[f"{run}_trie"], "PIN")).read().split()[0])
    except OSError: return None
def served():
    try: return [(a["height"], "tip" if a["trie_dir"] == D["tip_trie"] else "board") for a in json.load(open(os.path.join(D["serve"], "at.json")))]
    except OSError: return []
def proof():
    try: return json.load(open(os.path.join(D["serve"], "proof.rcpt")))["hi"]
    except OSError: return None
ENV = dict(os.environ, ZAPP_SERVE_DIR=D["serve"], COORD_SPINE=D["spine"], ZAPP_HOST=HOST, ZAPP_WORK=D["work"],
           BOARD_OUT=D["board_out"], BOARD_TRIE=D["board_trie"], TIP_FROM=str(X), TIP_OUT=D["tip_out"], TIP_TRIE=D["tip_trie"],
           ZAPP_PIN_W=str(W), ZAPP_OWNER="", ZAPP_RESTART=f"{ZS} {D['serve']}/at.json {STATUS}", ZAPP_ACTIVE=f"grep -q up {STATUS}",
           ZAPP_SETTLE_S="0")
def run():
    return subprocess.run(["bash", SCRIPT], env=ENV, capture_output=True, text=True, timeout=120)
fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1

prep("board", 1, 60, roots_from=40)
prep("tip", 1, 120, roots_from=90)
intact("board", 0); intact("tip", 0)
r = run()
check(r.returncode == 0 and not served() and "no spine" in r.stdout, f"1. no spine: nothing to do ({r.returncode})")
spine(30)
r = run()
check(r.returncode == 0 and not served() and pin("board") == 30, f"2. spine 30 (below the board pass's first root): not served, the board pass pinned at 30 ({r.returncode}, pin {pin('board')}; {r.stdout.strip()[-90:]})")
spine(50)
r = run()
check(r.returncode == 0 and served() == [(50, "board")] and proof() == 50 and pin("board") == 50,
      f"3. spine 50: served at [50] from the board pass, the proof published, pinned at 50 ({served()}, proof {proof()}, pin {pin('board')}; {r.stdout.strip()[-120:]} {r.stderr.strip()[-80:]})")
r = run()
check(r.returncode == 0 and "nothing new" in r.stdout, "   again: nothing new")
spine(55)
r = run()
check(r.returncode == 0 and served() == [(55, "board"), (50, "board")] and pin("board") == 50, f"4. spine 55: served at [55, 50], pinned at 50 ({served()}, pin {pin('board')})")
spine(99)
r = run()
check(r.returncode == 0 and served() == [(99, "tip"), (55, "board")] and proof() == 99 and pin("tip") == 99 and pin("board") == 55,
      f"5. spine 99 = X - 1: from the tip run, [99 tip, 55 board], both pinned ({served()}, pins {pin('tip')}/{pin('board')}; {r.stdout.strip()[-150:]} {r.stderr.strip()[-100:]})")
spine(110)
r = run()
check(r.returncode == 0 and served() == [(110, "tip"), (99, "tip")] and pin("tip") == 99 and pin("board") is None,
      f"6. spine 110: [110, 99] from the tip run, the tip run pinned at 99, the board pass unpinned ({served()}, pins {pin('tip')}/{pin('board')})")
# 7. a reorg: the tip run rewinds to 105 and preps branch B; the coordinator falls the spine back to the kept head 99
os.remove(os.path.join(D["spine"], "heads", "spine_110.json"))
prep("tip", 106, 120, tag="B")
spine(99, keep=False)
r = run()
check(r.returncode == 0 and served() == [(99, "tip")] and proof() == 99, f"7. a reorg, the spine back at 99: served at [99], 110 dropped ({served()}, {r.stdout.strip()[-100:]})")
# 8. advance on B, then fall back to 99 after it was released (past the window)
spine(112, tag="B")
r = run()
check(r.returncode == 0 and served() == [(112, "tip"), (99, "tip")], f"   the new branch: [112, 99] ({served()})")
intact("tip", 100)
spine(99, keep=False)
before = served()
r = run()
check(r.returncode == 1 and served() == before and "deeper than the window" in r.stdout, f"8. a fall-back to a released root: the run fails, nothing changes ({r.returncode}, {served()})")
intact("tip", 0)
# 9. a spine on another chain than the source's
spine(112, tag="Z")
open(os.path.join(D["spine"], "spine.bin"), "w").write(json.dumps({"hi": 112, "bh": bh("Z", 112)}))
r = run()
check(r.returncode == 1 and served() == before and "does not end on the source" in r.stdout, f"9. a spine not on the source's chain: fails, nothing changes ({r.returncode})")

# the side-by-side check before the switch (hazync-lz-zapp-compare.py): equal answers at the common height pass, one
# different answer fails
import http.server, threading
FACT = {"old": {}, "new": {}}
def server(name, tipj):
    class Hd(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            path, _, q = self.path.partition("?")
            body = json.dumps(tipj) if path == "/tip" else FACT[name].get(path, json.dumps({"fact": path}))
            if name == "new" and path != "/tip" and q != "at=50": body = json.dumps({"wrong height": q})
            b = body.encode(); self.send_response(200); self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
        def log_message(self, *a): pass
    sv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Hd)
    threading.Thread(target=sv.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{sv.server_address[1]}"
o, n = server("old", {"tip": 50}), server("new", {"tip": 55, "at": [50, 55]})
cmp_ = [sys.executable, os.path.join(HERE, "deploy/libzync/hazync-lz-zapp-compare.py"), "--old", o, "--new", n, "51ac"]
r = subprocess.run(cmp_, capture_output=True, text=True, timeout=60)
check(r.returncode == 0 and "3 of 3 facts equal at 50" in r.stdout, f"compare: equal answers at the common height 50 ({r.returncode}, {r.stdout.strip()[-60:]})")
FACT["new"]["/trie/history/51ac"] = json.dumps({"fact": "something else"})
r = subprocess.run(cmp_, capture_output=True, text=True, timeout=60)
check(r.returncode == 1 and "DIFFERENT /trie/history/51ac" in r.stdout, f"compare: one different answer fails ({r.returncode})")

shutil.rmtree(T, ignore_errors=True)
if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
