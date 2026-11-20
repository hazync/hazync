#!/usr/bin/env python3
"""libzync-prep-run.sh's start rules (coordinator/deploy/libzync/; REORG.md §19, review of D1 HIGH 1), with a stand-in
host_lz that records how it was started.

  1. a fresh root (nothing in cl/, trie/, out/), LZ_HI=follow: prep from genesis to `follow`
  2. no complete save point, but stores in cl/ and state files in out/: REFUSED, exit 3, the stores untouched, prep
     not started (it used to clear them and restart from genesis: weeks of the paper prep)
  3. the same with LZ_PREP_CLEAR_STORES=1: cleared, prep from genesis
  4. a `follow` run with stores and no save point, no state files: refused too
  5. a complete save point at 250 (the disk trie's meta at 250, its marker missing): LZ_RUN_DRYRUN=1 names it and changes
     nothing (no directory made, no file touched, prep not started); a start continues from it (HAZYNC_PREP_CONTINUE=1)
  6. LZ_HI that is neither a height nor `follow`: exit 2

  python3 test_prep_runner.py            # must PASS
  python3 test_prep_runner.py --control  # the refusal taken out (the runner before the review): MUST FAIL
"""
import json, os, shutil, stat, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = open(os.path.join(HERE, "deploy/libzync/libzync-prep-run.sh")).read()
CONTROL = "--control" in sys.argv[1:]
if CONTROL:
    old = 'if { out_has_states || [ "$HI" = follow ]; } && [ "${LZ_PREP_CLEAR_STORES:-0}" != 1 ]; then\n            echo "libzync-prep: REFUSED'
    assert SRC.count(old) == 1
    SRC = SRC.replace(old, 'if false; then\n            echo "libzync-prep: REFUSED')
    print("CONTROL: the runner without the refusal; the checks below MUST fail")
T = tempfile.mkdtemp(prefix="prep_runner_")
fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


def root(name):
    R = os.path.join(T, name)
    os.makedirs(os.path.join(R, "bin"))
    open(os.path.join(R, "bin/libzync-prep-run.sh"), "w").write(SRC)
    h = os.path.join(R, "bin/host_lz")
    open(h, "w").write("#!/bin/sh\necho \"$@ CONTINUE=${HAZYNC_PREP_CONTINUE:-}\" >> \"$(dirname \"$0\")/../host.log\"\n")
    os.chmod(h, os.stat(h).st_mode | stat.S_IEXEC)
    return R


def run(R, hi="follow", **env):
    e = {k: v for k, v in os.environ.items() if not k.startswith(("HAZYNC_", "LZ_"))}
    e.update(LZ_ROOT=R, LZ_HI=hi, **env)
    # output to files, not pipes: the runner's background snapshot pruner (a `sleep 60` loop) would hold a pipe open
    o, er = os.path.join(R, ".out"), os.path.join(R, ".err")
    with open(o, "w") as fo, open(er, "w") as fe:
        rc = subprocess.run(["bash", os.path.join(R, "bin/libzync-prep-run.sh")], env=e, stdout=fo, stderr=fe, timeout=60).returncode
    return subprocess.CompletedProcess([], rc, open(o).read(), open(er).read())


def started(R):
    try: return open(os.path.join(R, "host.log")).read().strip()
    except OSError: return ""


def listing(R):
    return sorted((d, tuple(sorted(x for x in f if x not in (".out", ".err")))) for d, _, f in os.walk(R))


R = root("fresh")
r = run(R)
check(r.returncode == 0 and started(R).startswith("libzync-prep 1 follow ") and " genesis " in started(R),
      f"1. a fresh follow run starts at genesis ({r.returncode}: {started(R)[:60]})")

R = root("lost")
for d in ("cl", "trie", "out"):
    os.makedirs(os.path.join(R, d))
open(os.path.join(R, "cl/L0.bin"), "wb").write(b"weeks of prep")
open(os.path.join(R, "out/state_5.json"), "w").write("{}")
r = run(R, hi="974936")
check(r.returncode == 3 and os.path.exists(os.path.join(R, "cl/L0.bin")) and not started(R) and "REFUSED" in r.stderr,
      f"2. no save point but stores and state files: refused with exit 3, the stores kept, prep not started ({r.returncode})")
r = run(R, hi="974936", LZ_RUN_DRYRUN="1")
check(r.returncode == 3 and "REFUSE" in r.stdout, f"   the dry run says so ({r.returncode}: {r.stdout.strip()[:70]})")
r = run(R, hi="974936", LZ_PREP_CLEAR_STORES="1")
check(r.returncode == 0 and not os.path.exists(os.path.join(R, "cl/L0.bin")) and " genesis " in started(R),
      f"3. with LZ_PREP_CLEAR_STORES=1: cleared, prep from genesis ({r.returncode})")

R = root("followlost")
os.makedirs(os.path.join(R, "trie"))
open(os.path.join(R, "trie/nodes.bin"), "wb").write(b"x")
r = run(R)
check(r.returncode == 3 and os.path.exists(os.path.join(R, "trie/nodes.bin")) and not started(R),
      f"4. a follow run with stores and no save point: refused ({r.returncode})")

R = root("saved")
for d in ("cl", "trie", "out"):
    os.makedirs(os.path.join(R, d))
open(os.path.join(R, "cl/L0.bin"), "wb").write(b"c")
meta = b"HZTRIED1" + struct.pack("<Q", 7) + b"\x01" + struct.pack("<Q", 250) + bytes(32) + struct.pack("<QQ", 9, 0)
open(os.path.join(R, "trie/meta"), "wb").write(meta)
for h in (249, 250, 251):
    open(os.path.join(R, f"out/state_{h}.json"), "w").write(json.dumps({"height": h}))
for x in ("smt", "coinlog"):
    open(os.path.join(R, f"out/state_250.{x}"), "w").write("m")
before = listing(R)
r = run(R, LZ_RUN_DRYRUN="1")
check(r.returncode == 0 and "would continue from state_250.json" in r.stdout and listing(R) == before and not started(R),
      f"5. the dry run names the save point 250 and changes nothing ({r.returncode}: {r.stdout.strip()[:70]})")
r = run(R)
check(r.returncode == 0 and started(R).startswith("libzync-prep 251 follow ") and "state_250.json" in started(R)
      and started(R).endswith("CONTINUE=1"), f"   a start continues from it ({started(R)[-60:]})")
r = run(root("bad"), hi="12x")
check(r.returncode == 2, f"6. LZ_HI=12x: exit 2 ({r.returncode})")

shutil.rmtree(T, ignore_errors=True)
if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if not fails else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
