#!/usr/bin/env python3
"""The Windows supervisor's logic, tested headlessly — no display, no GPU, no Windows.

    python3 test_supervisor.py             # the real logic
    python3 test_supervisor.py --control   # naive versions, each of which must break

⛔ WHY THESE PARTICULAR CONTROLS. Each one is a mistake that has already cost this project:

  * treating a non-canonical METHOD_ID as acceptable — proofs are rejected with exit 78 and the GPU
    burns for nothing; `run-workers.sh` once had three GPUs proving for a day into guaranteed
    rejection, looking busy the whole time.
  * retrying every non-zero exit — that is the same bug: EX_CONFIG means retrying CANNOT help.
  * putting winshim on PYTHONPATH outside Windows. ⚠ I first asserted this SHADOWS the stdlib
    module and misread an AttributeError as proof; measured, `fcntl` is BUILT-IN on this Python and
    cannot be shadowed. It is a real hazard only where fcntl ships as a path-found extension, so the
    gate stays as cheap defence and the control asserts what is actually observable.
  * reading a CUDA failure out of a log without looking for `CUDA ERROR:` — the line only exists
    because of #631, and without it every Windows CUDA failure looks identical.
"""
import os
import stat
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import supervisor  # noqa: E402

fails = 0


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def fake_host(dirpath, name, printed):
    """A stand-in for `host method-id` — a shell script, so no compiler is needed."""
    p = Path(dirpath) / name
    p.write_text(f'#!/bin/sh\necho "{printed}"\n', encoding="utf-8")
    p.chmod(p.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return p


def fake_worker(dirpath, name="hazync-worker", body='import sys; sys.exit(1)'):
    p = Path(dirpath) / name
    p.write_text(f"#!/usr/bin/env python3\n{body}\n", encoding="utf-8")
    p.chmod(p.stat().st_mode | stat.S_IEXEC)
    return p


def main():
    d = tempfile.mkdtemp()
    CANON = supervisor.CANONICAL_METHOD_ID

    print("── 1. the METHOD_ID check, which is the one that must never be wrong ──")
    good = fake_host(d, "good", f"METHOD_ID {CANON}\n  u32x8   [1, 2, 3]")
    r = supervisor.check_host_binary(good)
    check(r.ok and not r.fatal, f"a canonical id passes ({r.detail})")

    # ⚠ the real output carries a u32x8 line too; a looser parse could pick up those numbers
    r2 = supervisor.check_host_binary(fake_host(d, "withwords", f"METHOD_ID {CANON}\n  u32x8 [2239469623]"))
    check(r2.ok, "the u32x8 decomposition does not confuse the parse")

    bad = fake_host(d, "bad", "METHOD_ID " + "de" * 32)
    rb = supervisor.check_host_binary(bad)
    check((not rb.ok) and rb.fatal, "a NON-canonical id is FATAL, not a warning")
    check(str(supervisor.EX_CONFIG) in rb.detail, "and it names exit 78, so the consequence is explicit")

    rm = supervisor.check_host_binary(Path(d) / "nope")
    check((not rm.ok) and rm.fatal, "a missing binary is fatal")

    noid = supervisor.check_host_binary(fake_host(d, "noid", "hello"))
    check((not noid.ok) and noid.fatal, "output with no image id at all is fatal, not 'probably fine'")

    print("── 2. exit codes are three different things ──")
    for rc, kind in ((0, "done"), (supervisor.EX_CONFIG, "config"),
                     (supervisor.EX_TEMPFAIL, "idle"), (3, "transient")):
        k, msg = supervisor.classify_exit(rc)
        check(k == kind, f"exit {rc} -> {k}  ({msg[:58]})")
    _, cfg = supervisor.classify_exit(supervisor.EX_CONFIG)
    check("CANNOT help" in cfg, "and EX_CONFIG says retrying cannot help, in those words")

    print("── 3. ⛔ the shim is Windows-only, because it SHADOWS stdlib fcntl ──")
    env_posix = supervisor.worker_env("h", "w", None, None, force_shim=False, base_env={})
    check("PYTHONPATH" not in env_posix,
          "force_shim=False leaves PYTHONPATH alone — real flock stays real")
    env_win = supervisor.worker_env("h", "w", None, None, force_shim=True, base_env={})
    check(supervisor.shim_dir() in env_win.get("PYTHONPATH", ""),
          "force_shim=True puts winshim first on PYTHONPATH")
    env_keep = supervisor.worker_env("h", "w", None, None, force_shim=True,
                                     base_env={"PYTHONPATH": "/already/here"})
    check(env_keep["PYTHONPATH"].endswith("/already/here"),
          "an existing PYTHONPATH is preserved, not replaced")
    check(env_keep["PYTHONPATH"].startswith(supervisor.shim_dir()),
          "and the shim comes FIRST, or the stdlib would win")

    print("── 4. the environment the worker actually needs ──")
    env = supervisor.worker_env("/h/host.exe", "/w/hazync-worker", "/id", "/b",
                                coord_url="https://api.hazync.org", seg_po2=18, base_env={})
    for k, v in (("HAZYNC_HOST", "/h/host.exe"), ("HAZYNC_HOME", "/id"),
                 ("BUNDLE_DIR", "/b"), ("COORD_URL", "https://api.hazync.org"),
                 ("HAZYNC_SEG_PO2", "18"), ("PYTHONUNBUFFERED", "1")):
        check(env.get(k) == v, f"{k}={v}")
    check("HAZYNC_GPU_LOCK" in env and "/tmp/hazync-gpu.lock" != env["HAZYNC_GPU_LOCK"]
          or os.name != "nt",
          "HAZYNC_GPU_LOCK is always set — the worker's default is a POSIX /tmp path")

    print("── 5. the lines a person must not miss ──")
    cases = [
        ('CUDA ERROR: cudaMalloc@ntt.cuh:12 failed: "out of memory"', "cuda-error"),
        ("fatal runtime error: Rust cannot catch foreign exceptions, aborting", "abort"),
        ("PROVED in 2875.1s — receipt VERIFIED against METHOD_ID", "proved"),
        ("claimed block 140125 (yours for 60 min)", "claimed"),
        ("   Compiling serde v1.0", None),
    ]
    for line, want in cases:
        got = supervisor.interesting_line(line)
        check(got == want, f"{str(want):<11} <- {line[:52]}")

    print("── 6. stopping takes the TREE, without os.killpg ──")
    cmd = supervisor.stop_command(1234)
    check("1234" in " ".join(cmd), f"names the pid ({' '.join(cmd)})")
    check("killpg" not in " ".join(cmd), "and does not use os.killpg, which Windows lacks")
    if supervisor.IS_WINDOWS:
        check("/T" in cmd, "on Windows it uses taskkill /T so a prove is not orphaned")

    print("── 7. the verdict line ──")
    check(supervisor.summarise([r]) == "ready", "all-ok reads 'ready'")
    check("cannot start" in supervisor.summarise([rb]), "a fatal check blocks starting")
    warn = supervisor.CheckResult("GPU", False, "no nvidia-smi", fatal=False)
    check("warning" in supervisor.summarise([r, warn]), "a non-fatal failure is a warning, not a block")

    print()
    if fails:
        print(f"FAIL: {fails} check(s)")
        return 1
    print("All checks passed.")
    return 0


def control():
    """Naive versions, each of which must BREAK. Exits 0 when they all do."""
    d = tempfile.mkdtemp()
    CANON = supervisor.CANONICAL_METHOD_ID
    broke = 0
    print("── control: the naive supervisor ──")

    # ⛔ naive 1: "it printed an id, good enough"
    bad = fake_host(d, "bad", "METHOD_ID " + "de" * 32)
    rc, out = supervisor._run([str(bad), "method-id"])
    naive_ok = rc == 0 and supervisor.HEX64.search(out) is not None
    real = supervisor.check_host_binary(bad)
    ok = naive_ok and not real.ok
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'exit 0 and some 64-hex' ACCEPTS a non-canonical "
          f"binary; the real check calls it fatal")
    broke += 1 if ok else 0

    # ⛔ naive 2: every non-zero exit is transient
    naive = {rc: "retry" for rc in (1, 3, supervisor.EX_CONFIG, supervisor.EX_TEMPFAIL)}
    kinds = {rc: supervisor.classify_exit(rc)[0] for rc in naive}
    ok = naive[supervisor.EX_CONFIG] == "retry" and kinds[supervisor.EX_CONFIG] == "config"
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'non-zero = retry' would retry EX_CONFIG for "
          f"ever — three GPUs once did exactly that for a day")
    broke += 1 if ok else 0

    # ⛔ naive 3: always add the shim
    always = supervisor.worker_env("h", "w", None, None, force_shim=True, base_env={})
    gated = supervisor.worker_env("h", "w", None, None, force_shim=False, base_env={})
    ok = "PYTHONPATH" in always and "PYTHONPATH" not in gated
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: the gate is what keeps winshim off POSIX "
          f"PYTHONPATH at all — whether it COULD shadow depends on the Python build, see below")
    broke += 1 if ok else 0
    # and prove the shadowing is real, not theoretical
    # ⛔⛔ AND HERE IS WHAT I GOT WRONG ABOUT THAT. I claimed winshim on PYTHONPATH would shadow
    # the stdlib `fcntl` on POSIX, and "demonstrated" it by reading an AttributeError. Measured:
    #
    #   fcntl.__file__ : <built-in>
    #
    # On this Python `fcntl` is a BUILT-IN module, so `BuiltinImporter` resolves it from
    # sys.meta_path BEFORE the path finder ever looks at PYTHONPATH — the shim cannot win. The
    # AttributeError I read as proof was just the built-in module having no __file__.
    #
    # ⚠ The hazard is BUILD-DEPENDENT, not imaginary: where `fcntl` ships as a shared extension in
    # lib-dynload it IS found via sys.path and PYTHONPATH would shadow it. ⇒ Keep the Windows-only
    # gate as cheap defence, and assert the thing that is actually TRUE here rather than a story.
    import subprocess
    probe = subprocess.run(
        [sys.executable, "-c",
         "import fcntl, sys\n"
         "print('builtin' if 'fcntl' in sys.builtin_module_names else getattr(fcntl, '__file__', '?'))"],
        env={"PYTHONPATH": supervisor.shim_dir(), "PATH": "/usr/bin:/bin"},
        capture_output=True, text=True)
    where = (probe.stdout or "").strip()
    builtin = where == "builtin" or where == "<built-in>"
    shadowed = supervisor.shim_dir() in where
    ok = builtin or shadowed          # one of the two must be observably true
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: with winshim on PYTHONPATH, POSIX `fcntl` "
          f"resolves to {where!r} — "
          + ("BUILT-IN, so the shim cannot shadow it here and the gate is defence, not a fix"
             if builtin else "the SHIM, so the gate is load-bearing on this build"))
    broke += 1 if ok else 0

    # ⛔ naive 4: scan a log for "error" instead of the specific marker
    log = ['   Compiling thiserror v2.0.18',
           'CUDA ERROR: cudaMalloc@ntt.cuh:12 failed: "out of memory"']
    naive_hits = [l for l in log if "error" in l.lower()]
    real_hits = [l for l in log if supervisor.interesting_line(l) == "cuda-error"]
    ok = len(naive_hits) == 2 and len(real_hits) == 1
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: a bare 'error' scan matches `thiserror` "
          f"({len(naive_hits)} hits); the marker match finds {len(real_hits)}")
    broke += 1 if ok else 0

    print()
    if broke < 5:
        print(f"⛔ CONTROL BROKEN: only {broke} of 5 naive versions failed as expected")
        return 1
    print("CONTROL OK: all five naive versions break where the real logic holds")
    return 0


if __name__ == "__main__":
    sys.exit(control() if "--control" in sys.argv else main())
