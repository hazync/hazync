"""The installed Hazync.exe runs the worker and the setup checks as python.exe would (frozenpy.run_as_python).

    python test_frozenpy.py            # must PASS
    python test_frozenpy.py --control  # run_as_python reduced to "always open the window": MUST FAIL
"""
import os
import subprocess
import sys
import tempfile

import frozenpy

CONTROL = "--control" in sys.argv
if CONTROL:
    frozenpy.run_as_python = lambda argv=None: None
    print("CONTROL: the dispatch taken out -- the checks below MUST fail")

fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


T = tempfile.mkdtemp(prefix="frozenpy_")
script = os.path.join(T, "hazync-worker")          # no .py, as the release asset
out = os.path.join(T, "out.txt")
with open(script, "w") as f:
    f.write("import sys\nopen(sys.argv[0] + '.ran', 'w').write(' '.join(sys.argv[1:]))\n"
            "import fcntl_probe\nsys.exit(int(sys.argv[2]) if len(sys.argv) > 2 else 0)\n")
shim = os.path.join(T, "shim")
os.makedirs(shim)
open(os.path.join(shim, "fcntl_probe.py"), "w").write("X = 1\n")

sys.frozen = True
try:
    os.environ["PYTHONPATH"] = shim
    rc = frozenpy.run_as_python(["Hazync.exe", script, "run", "7"])
    check(rc == 7 and open(script + ".ran").read() == "run 7",
          f"Hazync.exe <worker> run 7: the script ran with its arguments and its exit status ({rc})")
    check(shim in sys.path, "PYTHONPATH's entries reach sys.path (the worker's fcntl shim)")
    rc = frozenpy.run_as_python(["Hazync.exe", "-c", f"open({out!r}, 'w').write('hi'); import sys; sys.exit(3)"])
    check(rc == 3 and open(out).read() == "hi", f"Hazync.exe -c <code>: the code ran, status 3 ({rc})")
    rc = frozenpy.run_as_python(["Hazync.exe", "-c", "raise ValueError('x')"])
    check(rc == 1, f"an exception in the code is status 1, not a crash ({rc})")
    check(frozenpy.run_as_python(["Hazync.exe"]) is None, "no arguments: the window opens")
    check(frozenpy.run_as_python(["Hazync.exe", "--debug"]) is None, "an option, not code: the window opens")
    check(frozenpy.run_as_python(["Hazync.exe", os.path.join(T, "missing")]) is None,
          "a file that is not there: the window opens")
finally:
    del sys.frozen
if not CONTROL:
    check(frozenpy.run_as_python(["python", script, "run"]) is None, "from source (not frozen): never dispatches")

# a windowed build starts with no stdout: the pipe the app reads must still get the output
code = ("import sys, frozenpy; sys.frozen = True; sys.stdout = None; "
        f"raise SystemExit(frozenpy.run_as_python(['Hazync.exe', '-c', 'print(\"piped\")']))")
if CONTROL:
    code = "import sys; sys.stdout = None; raise SystemExit(0)"
p = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                   cwd=os.path.dirname(os.path.abspath(__file__)))
check(p.returncode == 0 and p.stdout.strip() == "piped", f"with sys.stdout None, output still reaches the pipe ({p.stdout!r})")

if CONTROL:
    if fails >= 4:
        print(f"CONTROL OK: {fails} checks fail without the dispatch, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
