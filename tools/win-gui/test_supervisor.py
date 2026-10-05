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

import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

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
    """A stand-in for `host method-id`. ⚠ A .bat on Windows, a shell script on POSIX.

    ⛔ The first version wrote `#!/bin/sh` everywhere, which Windows cannot execute — so five
    checks "failed" on Windows against code that was perfectly fine. A fixture that only works on
    one platform turns a portability test into noise.
    """
    if os.name == "nt":
        p = Path(dirpath) / (name + ".bat")
        body = "@echo off\r\n" + "".join(f"echo {ln}\r\n" for ln in printed.splitlines())
        p.write_text(body, encoding="utf-8")
        return p
    p = Path(dirpath) / name
    p.write_text(f'#!/bin/sh\ncat <<\'EOF\'\n{printed}\nEOF\n', encoding="utf-8")
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

    print("── 7. known failures are translated, not echoed ──")
    # ⛔ Every one of these is a string this project actually hit. A newcomer meeting the same text
    # has none of the context that made it diagnosable.
    for text, must in [
        ("fatal runtime error: Rust cannot catch foreign exceptions, aborting", "discarded"),
        # ⚠ Was pinned to the phrase "real GPU error". explain() now QUOTES the GPU's own text
        # instead of describing it, which is more useful and is what this should assert: the
        # verbatim error, including where in the kernel it came from.
        ('CUDA ERROR: cudaMalloc@ntt.cuh:12 failed: "out of memory"', "cudaMalloc@ntt.cuh:12"),
        ("thread panicked: out of memory", "HAZYNC_SEG_PO2"),
        ("No module named 'fcntl'", "bug in this program"),
    ]:
        got = supervisor.explain(text)
        check(bool(got) and must in got, f"{text[:44]:46} -> mentions {must!r}")
    check(supervisor.explain("   Compiling serde v1.0") is None,
          "and ordinary output gets no invented explanation")

    print("── 8. a diagnostic verdict reads the OUTPUT, not the exit code ──")
    cases = [
        ("method-id", 0, "METHOD_ID " + supervisor.CANONICAL_METHOD_ID, True),
        ("method-id", 0, "METHOD_ID " + "de" * 32, False),
        ("method-id", 0, "", False),
        ("regress", 0, ">>> REGRESSION PASS ✓", True),
        # ⛔ THE ONE THAT MATTERS: exit 0 with no pass line must FAIL. A check that passes on
        # silence is not a check, and this project has been bitten by exactly that shape.
        ("regress", 0, "started, then nothing", False),
        ("prove-block", 0, "PROVED in 2875.1s — receipt VERIFIED against METHOD_ID", True),
        ("prove-block", 134, "fatal runtime error: Rust cannot catch foreign exceptions", False),
    ]
    for name, rc, out, want in cases:
        ok, msg = supervisor.diagnostic_verdict(name, rc, out)
        check(ok == want, f"{name:12} rc={rc:<4} -> {'pass' if ok else 'FAIL'}  ({msg[:44]})")
    _, msg = supervisor.diagnostic_verdict("prove-block", 0,
                                           "PROVED in 2875.1s — receipt VERIFIED")
    check("2875.1" in msg, f"and a successful prove reports its TIME ({msg})")

    print("── 9. the verdict line ──")
    check(supervisor.summarise([r]) == "ready", "all-ok reads 'ready'")
    check("cannot start" in supervisor.summarise([rb]), "a fatal check blocks starting")
    warn = supervisor.CheckResult("GPU", False, "no nvidia-smi", fatal=False)
    check("warning" in supervisor.summarise([r, warn]), "a non-fatal failure is a warning, not a block")

    print("── the worker must be able to PRINT ITS OWN HELP on a cp1252 machine ──")
    # ⛔⛔ MEASURED ON WINDOWS 2026-10-05. The client downloaded correctly and was then rejected as "it
    # does not run", because the first thing check_worker asks it to do is print its help -- and the
    # worker's docstring contains ⛔ (U+26D4), which cp1252 cannot encode. It died with
    # UnicodeEncodeError before doing anything. The worker is a release asset used exactly as it ships,
    # so the only place to fix it is the environment we hand it.
    env = supervisor.worker_env(host_path=None, worker_path=None, identity_dir=None, bundle_dir=None)
    check(env.get("PYTHONIOENCODING") == "utf-8",
          f"worker_env sets PYTHONIOENCODING=utf-8 (got {env.get('PYTHONIOENCODING')!r})")

    # ⭐ PROVE THE PREMISE rather than assert it: cp1252 really cannot encode that character, and utf-8
    # really can. If a future Windows defaults to utf-8, this stops being load-bearing and should say so.
    try:
        "\u26d4".encode("cp1252")
        check(False, "cp1252 unexpectedly encoded U+26D4 — this assertion is no longer load-bearing")
    except UnicodeEncodeError:
        check(True, "cp1252 genuinely cannot encode U+26D4, which is why the variable is needed")
    check("\u26d4".encode("utf-8") == b"\xe2\x9b\x94", "and utf-8 can")

    # and the decode side must match, or the fix turns into mojibake
    import inspect
    rsrc = inspect.getsource(supervisor._run)
    check('encoding="utf-8"' in rsrc,
          "_run decodes as utf-8 too — text=True alone uses the LOCALE encoding, i.e. cp1252 on Windows")

    print("── a failing --help must report the END of the traceback ──")
    # ⛔ A Python traceback puts the EXCEPTION last; its first 160 characters are the word "Traceback"
    # and a file path. Showing the head is showing everything except the answer.
    csrc = inspect.getsource(supervisor.check_worker)
    check("[-6:]" in csrc or "splitlines()[-" in csrc,
          "check_worker reports the LAST lines of the output, not the first")
    check("[:160]" not in csrc,
          "and no longer truncates to the first 160 characters, which hid the real error")

    print("── which checks may run themselves ──")
    # ⭐ Asked directly: "all of these checks should be automatic surely on startup?" -- and for two
    # of the three the answer is yes. The distinction is COST, so it is asserted rather than left to
    # whoever edits the list next.
    auto = supervisor.AUTO_DIAGNOSTICS
    check("method-id" in auto and "regress" in auto,
          f"the two cheap checks run automatically ({auto})")
    check("prove-block" not in auto,
          "⛔ the 2,875 s prove does NOT — starting it uninvited pins the machine for ~an hour")
    # ⚠ A timeout is the honest measure of what a check costs. Anything allowed to run at startup
    # must be bounded by something a person would not notice; this catches a future check being
    # flagged auto= with an hour-long budget.
    slow = [d[0] for d in supervisor.DIAGNOSTICS if d[4] and d[3] > 600]
    check(not slow, f"nothing automatic has a timeout over 10 minutes{f' — {slow}' if slow else ''}")
    check(all(len(d) == 5 for d in supervisor.DIAGNOSTICS),
          "every diagnostic declares (name, title, note, timeout, auto)")

    print("── ⛔⛔ a real CUDA ERROR line must outrank the abort that follows it ──")
    # The 2026-10-04 build prints the GPU's own error AND THEN still aborts, so BOTH strings appear
    # in one output. First-match ordering explained the abort — "if there is none, you are on an
    # older binary" — which is exactly backwards when the line IS there. It would have told the one
    # person running the new build that they were on the old one, and discarded the only evidence
    # hazync#631 has ever produced.
    both = ("=== PROVING block 170 chain_step (real STARK receipt) ===\n"
            "CUDA ERROR: out of memory\n"
            "fatal runtime error: Rust cannot catch foreign exceptions, aborting")
    check(supervisor.cuda_error_text(both) == "out of memory",
          f"the GPU's own message is extracted ({supervisor.cuda_error_text(both)!r})")
    why = supervisor.explain(both) or ""
    check("out of memory" in why, "explain() quotes the real error")
    check("older binary" not in why,
          "⛔ and does NOT claim they are on an older binary when the line is present")
    check("HAZYNC_SEG_PO2" in why, "and names the knob for this particular error")

    # an unknown CUDA error must still be reported verbatim rather than guessed at
    odd = "CUDA ERROR: unspecified launch failure\nfatal runtime error: Rust cannot catch foreign exceptions"
    why2 = supervisor.explain(odd) or ""
    check("unspecified launch failure" in why2, "an unfamiliar GPU error is quoted, not swallowed")

    # ⚠ and the OLD build, which prints no such line, must keep its original explanation
    old_only = "fatal runtime error: Rust cannot catch foreign exceptions, aborting"
    check(supervisor.cuda_error_text(old_only) is None, "no CUDA line in old output")
    check("older binary" in (supervisor.explain(old_only) or ""),
          "the old build is still told it is the old build")

    print("── ⛔⛔ hazync#631: an unsupported GPU is not a broken driver ──")
    # 📏 Measured 2026-10-05, GTX 1050 Ti, with the build that finally prints the GPU's own error:
    #   cudaErrorNoDevice@sppark/util/all_gpus.cpp:43 — "no CUDA-capable device is detected"
    # on a machine where nvidia-smi lists the card. all_gpus.cpp keeps only prop.major >= 7, sppark
    # sets that to "Volta and forward", and a 1050 Ti is 6.1 — filtered out, list empty, synthetic
    # error. ⭐ The same card fails identically on Linux: this was never a Windows bug.
    nodev = ('CUDA ERROR: cudaErrorNoDevice@sppark/util/all_gpus.cpp:43 failed: '
             '"no CUDA-capable device is detected"')
    why = supervisor.explain(nodev) or ""
    check("compute capability" in why, "explain() names compute capability, not a driver fault")
    check("7.0" in why or "7" in why, "and the floor that applies")
    check("CPU build" in why, "and what to do instead")
    check("out of memory" not in why.lower(),
          "⛔ and does NOT send someone to lower HAZYNC_SEG_PO2 — VRAM is irrelevant here")

    print("── a long check must show progress while it runs ──")
    # ⛔⛔ Reported from a real machine: "trying to prove via cpu but there isn't any form of
    # progress being shown". _run returns everything only when the process EXITS, so a 2,875 s
    # prove showed nothing for forty minutes — and the prover is not silent, it prints
    # "0/2 segments  10s elapsed, ~0s left" as it goes. A long job that shows nothing is
    # indistinguishable from a hung one.
    import time as _t
    seen = []
    t0 = _t.time()
    rc, out = supervisor.run_stream(
        [sys.executable, "-u", "-c",
         "import time,sys\nfor i in range(3):\n print(f'  {i}/3 segments', flush=True)\n"
         " time.sleep(0.3)"],
        timeout=30, on_line=lambda l: seen.append((_t.time() - t0, l)))
    check(rc == 0, f"run_stream returns the exit code (rc={rc})")
    check(len(seen) == 3, f"every line is delivered ({len(seen)})")
    check(all(l in out for _, l in seen), "and the full output is still returned for the verdict")
    # ⭐ The point is WHEN they arrive, not that they arrive. Buffered output would land together.
    spread = (seen[-1][0] - seen[0][0]) if len(seen) > 1 else 0
    check(spread > 0.3,
          f"lines arrive as the process runs, not in one lump at the end ({spread:.1f}s apart)")

    # ⚠ The timeout must fire on a SILENT process — the case a timeout exists for. A check in the
    # read loop cannot, because the loop is blocked waiting for a line that never comes.
    t1 = _t.time()
    rc2, _ = supervisor.run_stream([sys.executable, "-u", "-c", "import time; time.sleep(30)"],
                                   timeout=2)
    took = _t.time() - t1
    check(took < 10, f"a silent process is killed by the watchdog ({took:.1f}s, not 30s)")
    check(rc2 != 0, f"and that is reported as a failure (rc={rc2})")

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

    # ⛔ naive 5: judge a diagnostic by its exit code
    out = "started, then nothing"
    naive = (0 == 0)                       # "exit 0, so it passed"
    real, _ = supervisor.diagnostic_verdict("regress", 0, out)
    ok = naive and not real
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'exit 0 means it passed' calls a regress that "
          f"printed NO pass line a success — a check that passes on silence is not a check")
    broke += 1 if ok else 0

    print()
    if broke < 6:
        print(f"⛔ CONTROL BROKEN: only {broke} of 6 naive versions failed as expected")
        return 1
    print("CONTROL OK: all six naive versions break where the real logic holds")
    return 0


if __name__ == "__main__":
    sys.exit(control() if "--control" in sys.argv else main())
