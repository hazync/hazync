#!/usr/bin/env python3
"""The worker on an engine host (engine/STAGE3.md 3.2): `host seg-po2` answers `engine`, and a failed prove is
tried ONCE, not walked down a ladder of segment sizes the engine does not have (each rung would be the same run).

  1. `engine` from seg-po2: the ladder is a single attempt; a failing prove runs the host exactly once;
  2. a number from seg-po2 (a RISC Zero host): the ladder still walks down to 18 (unchanged);
  3. the worker asks seg-po2 with the prove's environment (HAZYNC_ENGINE is what makes the host say `engine`).

  python3 test_worker_engine.py            # must PASS
  python3 test_worker_engine.py --control  # the engine answer ignored; check 1 MUST FAIL
"""
import importlib.machinery
import importlib.util
import os
import stat
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ["HAZYNC_HOME"] = tempfile.mkdtemp(prefix="hz_eng_")
CONTROL = "--control" in sys.argv

loader = importlib.machinery.SourceFileLoader("hazync_cli", os.path.join(HERE, "hazync"))
spec = importlib.util.spec_from_loader("hazync_cli", loader)
hz = importlib.util.module_from_spec(spec)
loader.exec_module(hz)

T = tempfile.mkdtemp(prefix="hz_eng_host_")
FAILS = []


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        FAILS.append(what)


def fake_host(name):
    """seg-po2: `engine` when HAZYNC_ENGINE is set, else 21; every prove fails (an ordinary error); counts runs."""
    p = os.path.join(T, name)
    count = os.path.join(T, name + ".count")
    with open(p, "w") as f:
        f.write(f"""#!/bin/bash
case "$1" in
  seg-po2) if [ -n "$HAZYNC_ENGINE" ]; then echo engine; else echo 21; fi; exit 0 ;;
  method-id) echo "METHOD_ID {'ab' * 32}"; exit 0 ;;
esac
echo x >> {count}
echo "block INVALID: some ordinary failure" >&2
exit 1
""")
    os.chmod(p, os.stat(p).st_mode | stat.S_IEXEC)
    return p, count


def runs(count):
    try:
        return len(open(count).read().split())
    except OSError:
        return 0


def ladder(host, env):
    hz.HOSTBIN = host
    hz._SEG_TOP_CACHE["v"] = False
    if CONTROL:
        env = {k: v for k, v in env.items() if k != "HAZYNC_ENGINE"}
    return hz._seg_ladder(env)


# 1. the engine host
host, count = fake_host("enghost")
env = dict(os.environ, HAZYNC_ENGINE="/bin/hzengine-stub")
check(ladder(host, env) == (None,), f"an engine host's ladder is one attempt (got {ladder(host, env)})")
hz.HOSTBIN = host
hz._SEG_TOP_CACHE["v"] = False
if CONTROL:
    env = {k: v for k, v in env.items() if k != "HAZYNC_ENGINE"}
try:
    hz._run_with_seg_retry(["prove-range-bridge", "1"], env, T, "block 1")
except SystemExit:
    pass
check(runs(count) == 1, f"a failing prove on the engine runs the host once (ran {runs(count)} times)")

# 2. a RISC Zero host: unchanged
host2, count2 = fake_host("r0host")
env2 = {k: v for k, v in os.environ.items() if k != "HAZYNC_ENGINE"}
want = (None, 20, 19, 18)
got = ladder(host2, env2) if not CONTROL else (None, 20, 19, 18)
check(got == want, f"a RISC Zero host's ladder walks 21 -> 18 as before (got {got})")

print()
if FAILS:
    print(f"FAILED: {len(FAILS)} check(s)")
    sys.exit(1)
print("the worker makes one attempt on an engine host and walks the ladder on a RISC Zero one")
