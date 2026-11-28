#!/usr/bin/env python3
"""The worker on an engine host (engine/STAGE3.md 3.2): `host seg-po2` answers `engine`, and a failed prove is
tried ONCE, not walked down a ladder of segment sizes the engine does not have (each rung would be the same run).

  1. `engine` from seg-po2: the ladder is a single attempt; a failing prove runs the host exactly once;
  2. a number from seg-po2 (a RISC Zero host): the ladder still walks down to 18 (unchanged);
  3. the worker asks seg-po2 with the prove's environment (HAZYNC_ENGINE is what makes the host say `engine`);
  4. an engine box that cannot prove at all (hzengine cannot load libcudart.so.12; no key directory) stops the worker
     with EX_CONFIG on the first attempt, so run-workers.sh stops the loop instead of claiming block after block;
     an ordinary failure does not.

  python3 test_worker_engine.py            # must PASS
  python3 test_worker_engine.py --control  # the engine answer and the engine box errors ignored; checks 1, 4 MUST FAIL
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


def fake_host(name, err="block INVALID: some ordinary failure"):
    """seg-po2: `engine` when HAZYNC_ENGINE is set, else 21; every prove fails with `err`; counts runs."""
    p = os.path.join(T, name)
    count = os.path.join(T, name + ".count")
    with open(p, "w") as f:
        f.write(f"""#!/bin/bash
case "$1" in
  seg-po2) if [ -n "$HAZYNC_ENGINE" ]; then echo engine; else echo 21; fi; exit 0 ;;
  method-id) echo "METHOD_ID {'ab' * 32}"; exit 0 ;;
esac
echo x >> {count}
echo "{err}" >&2
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

# 4. an engine box that cannot prove: EX_CONFIG at once
if CONTROL:
    hz._DEVICE_FATAL = tuple(h for h in hz._DEVICE_FATAL
                             if h not in ("error while loading shared libraries", "HAZYNC_ENGINE_KEYS: the engine's key directory"))
for i, (name, msg, fatal) in enumerate((
        ("no libcudart.so.12", "/workspace/hzengine: error while loading shared libraries: libcudart.so.12: cannot open shared object file: "
         "No such file or directory", True),
        ("no key directory", "thread 'main' panicked at src/engine_backend.rs:41:62:\nHAZYNC_ENGINE_KEYS: the engine's key directory: "
         "NotPresent", True),
        ("ordinary", "block INVALID: bad-txns-inputs-missingorspent", False))):
    h, c = fake_host(f"engbox{i}", msg)
    hz.HOSTBIN = h
    hz._SEG_TOP_CACHE["v"] = False
    env4 = dict(os.environ, HAZYNC_ENGINE="/bin/hzengine-stub")
    code = None
    try:
        hz._run_with_seg_retry(["prove-range-bridge", "1"], env4, T, "block 1")
    except SystemExit as e:
        code = e.code
    if fatal:
        check(code == hz.EX_CONFIG and runs(c) == 1,
              f"{name}: the worker stops with EX_CONFIG after one run (exit {code}, {runs(c)} runs)")
    else:
        check(code != hz.EX_CONFIG, f"an ordinary block failure does not stop the box (exit {str(code)[:40]})")

# 5. review R3-6 #4 (28.11.2026): run-workers.sh on an engine box needs the release's key pin; its smoke's verify-any runs
#    under HAZYNC_ENGINE_VK + _PIN, never an unpinned key directory. Refused before anything is started.
import hashlib  # noqa: E402
import shutil  # noqa: E402
import subprocess  # noqa: E402
rw = tempfile.mkdtemp(prefix="hz_eng_rw_")
shutil.copy(os.path.join(HERE, "run-workers.sh"), rw)
shutil.copy(os.path.join(HERE, "hazync"), rw)
keys = os.path.join(rw, "keys")
os.makedirs(keys)
for n in ("sdk.pk", "agg.vk", "openvm.toml", "guest.vmexe", "baseline.json", "vk.bin"):
    with open(os.path.join(keys, n), "w") as f:
        f.write(n + "\n")
eng = os.path.join(rw, "hzengine")
host = os.path.join(rw, "host")
for p_, body in ((eng, "#!/bin/bash\n"), (host, "#!/bin/bash\n[ \"$1\" = method-id ] && echo \"METHOD_ID " + "e3" * 32 + "\"\nexit 0\n")):
    with open(p_, "w") as f:
        f.write(body)
    os.chmod(p_, os.stat(p_).st_mode | stat.S_IEXEC)
good_pin = hashlib.sha256(b"vk.bin\n").hexdigest()
base_env = dict(os.environ, HAZYNC_HOST=host, HAZYNC_ENGINE=eng, HAZYNC_ENGINE_KEYS=keys, COORD_URL="http://127.0.0.1:9",
                LOG_DIR=os.path.join(rw, "logs"), SKIP_GPU_SMOKE="1")
base_env.pop("HAZYNC_ENGINE_VK", None)
base_env.pop("HAZYNC_ENGINE_VK_PIN", None)


def rw_run(**extra):
    r = subprocess.run(["bash", os.path.join(rw, "run-workers.sh"), "1"], env=dict(base_env, **extra),
                       capture_output=True, text=True, timeout=120)
    subprocess.run(["bash", os.path.join(rw, "run-workers.sh"), "1", "--stop"], env=dict(base_env, **extra),
                   capture_output=True, text=True, timeout=60)
    return r


r = rw_run()
check(r.returncode != 0 and "HAZYNC_ENGINE_VK_PIN" in r.stderr and "engine          :" not in r.stdout,
      "run-workers: an engine box with no key pin is refused before anything starts (R3-6 #4)")
r = rw_run(HAZYNC_ENGINE_VK_PIN="00" * 32)
check(r.returncode != 0 and "is not the pinned key" in r.stderr, "run-workers: a pin that is not the key file's is refused")
r = rw_run(HAZYNC_ENGINE_VK_PIN=good_pin)
check("engine          :" in r.stdout and "key pin " + good_pin[:16] in r.stdout,
      "run-workers: with the release's pin the engine box passes the pre-flight")

print()
if FAILS:
    print(f"FAILED: {len(FAILS)} check(s)")
    sys.exit(1)
print("the worker makes one attempt on an engine host and walks the ladder on a RISC Zero one")
