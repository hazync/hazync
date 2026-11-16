#!/usr/bin/env python3
"""The reorg STOP path (docs/libzync/REORG.md §11, phase P2): exit 4 must FAIL the prep and fetch units, not restart
them, so the alert.conf drop-in's OnFailure= fires.

⛔ WHY. A unit that restarts for ever never enters `failed`, so OnFailure= never says a word (the 2026-09 crash-loop
lesson in hazync-alert.sh). Before P2 a reorg made libzync-prep panic and restart every 30 s with nobody told.

  python3 test_reorg_units.py               # static checks, the run script, AND the real units under `systemctl --user`
  python3 test_reorg_units.py --no-systemd  # without a systemd user session (CI): static checks and the run script only
  python3 test_reorg_units.py --control     # the units without RestartPreventExitStatus=4: MUST FAIL

1. the four units (libzync-{,board-}{prep,fetch}) say Restart=on-failure and RestartPreventExitStatus has 4 (and 3 for
   prep), and each has an alert drop-in with OnFailure=hazync-alert@%n.service;
2. libzync-prep-run.sh passes the host's exit status through (4 stays 4);
3. under systemd, each unit copied verbatim (LZ_ROOT moved to a scratch dir by a drop-in, the alert template replaced by
   one that writes a file): a stop for a reorg leaves it `failed` with status 4, no restart, and the alert fired; an
   ordinary failure (exit 1) still restarts and does not alert.
"""
import os, pathlib, re, shutil, subprocess, sys, tempfile, time

REPO = pathlib.Path(__file__).resolve().parent.parent
DEPLOY = REPO / "coordinator" / "deploy"
UNITS = {"libzync-prep": "34", "libzync-board-prep": "34", "libzync-fetch": "4", "libzync-board-fetch": "4"}
CONTROL = "--control" in sys.argv
SYSTEMD = "--no-systemd" not in sys.argv

fails, ran = [], []
def check(ok, what):
    ran.append(what)
    print(f"  {'ok  ' if ok else 'FAIL'} {what}", flush=True)
    if not ok: fails.append(what)

def unit_text(u):
    t = (DEPLOY / "libzync" / f"{u}.service").read_text()
    # the control: the unit as it was before P2 (no exit 4 in RestartPreventExitStatus)
    return re.sub(r"^RestartPreventExitStatus=.*$", lambda m: "RestartPreventExitStatus=3" if "3" in m.group(0) else "",
                  t, flags=re.M) if CONTROL else t

def directive(text, key):
    return [l.split("=", 1)[1].strip() for l in text.splitlines() if l.startswith(key + "=")]

print("== 1. the units and their alert drop-ins")
for u, want in UNITS.items():
    t = unit_text(u)
    rp = " ".join(directive(t, "RestartPreventExitStatus")).split()
    check(directive(t, "Restart") == ["on-failure"] and sorted(rp) == sorted(want),
          f"{u}: Restart=on-failure, RestartPreventExitStatus={' '.join(sorted(want))} (has {rp})")
    a = DEPLOY / "dropins" / f"{u}-alert.conf"
    check(a.exists() and directive(a.read_text(), "OnFailure") == ["hazync-alert@%n.service"], f"{u}: dropins/{a.name} has OnFailure=hazync-alert@%n.service")
check((DEPLOY / "hazync-alert@.service").exists(), "the alert template unit is in the repo")

def scratch_root(host_rc):
    r = pathlib.Path(tempfile.mkdtemp(prefix="reorg-units-"))
    (r / "bin").mkdir()
    shutil.copy(DEPLOY / "libzync" / "libzync-prep-run.sh", r / "bin")
    shutil.copy(REPO / "prover" / "fetch_block_rpc.py", r / "bin")
    host = r / "bin" / "host_lz"
    host.write_text(f'#!/bin/sh\necho "$@" > "$(dirname "$0")/../host_args"\nexit {host_rc}\n')
    host.chmod(0o755)
    (r / "bin" / "libzync-prep-run.sh").chmod(0o755)
    return r

print("== 2. libzync-prep-run.sh passes the host's exit status through")
for rc in (0, 3, 4):
    r = scratch_root(rc)
    # output to a file, not a pipe: the script's background pruner (a `sleep 60` loop) keeps a pipe open after it exits
    with open(r / "run.log", "w") as log:
        got = subprocess.run(["bash", str(r / "bin" / "libzync-prep-run.sh")], env=dict(os.environ, LZ_ROOT=str(r), LZ_HI="10"),
                             stdout=log, stderr=subprocess.STDOUT, timeout=60)
    args = (r / "host_args").read_text().split() if (r / "host_args").exists() else []
    check(got.returncode == rc and args[:3] == ["libzync-prep", "1", "10"] and args[-1] == f"{r}/out",
          f"run script: host exit {rc} -> {got.returncode} (host called with {' '.join(args[:3])} ...)")
    shutil.rmtree(r, ignore_errors=True)

def systemd_case(u, kind, tag):
    """kind 'reorg': the host exits 4 (prep) / a REORG marker is in blocks/ (fetch). kind 'crash': exit 1."""
    prep = u.endswith("prep")
    r = scratch_root(4 if kind == "reorg" else 1)
    (r / "blocks").mkdir()
    if not prep and kind == "reorg": (r / "blocks" / "REORG").write_text("{}\n")
    ud = pathlib.Path(os.environ.get("XDG_CONFIG_HOME", pathlib.Path.home() / ".config")) / "systemd" / "user"
    name, alert = f"{tag}-{u}", f"{tag}-alert"
    (ud / f"{name}.service.d").mkdir(parents=True, exist_ok=True)
    (ud / f"{name}.service").write_text(unit_text(u))
    (ud / f"{name}.service.d" / "alert.conf").write_text(
        (DEPLOY / "dropins" / f"{u}-alert.conf").read_text().replace("hazync-alert@", f"{alert}@"))
    # LZ_ROOT moved to the scratch dir; the fetcher's RPC pointed at nothing (exit 1 if it gets that far)
    (ud / f"{name}.service.d" / "zz-test.conf").write_text(
        f"[Service]\nEnvironment=LZ_ROOT={r} LZ_HI=10 HAZYNC_RPC_COOKIE={r}/no-cookie\n")
    (ud / f"{alert}@.service").write_text(f"[Service]\nType=oneshot\nExecStart=/bin/sh -c 'echo %i >> {r}/alerts'\n")
    sc = lambda *a: subprocess.run(["systemctl", "--user", *a], capture_output=True, text=True, timeout=60)
    try:
        sc("daemon-reload")
        sc("start", "--no-block", f"{name}.service")
        show = {}
        end = time.time() + 30
        while time.time() < end:
            show = dict(l.split("=", 1) for l in sc("show", f"{name}.service", "-p", "ActiveState,SubState,Result,ExecMainStatus,NRestarts").stdout.split())
            if show.get("ActiveState") == "failed" or show.get("SubState") == "auto-restart": break
            time.sleep(0.2)
        time.sleep(1.0)                               # the OnFailure= oneshot
        alerts = (r / "alerts").read_text().split() if (r / "alerts").exists() else []
        if kind == "reorg":
            check(show.get("ActiveState") == "failed" and show.get("ExecMainStatus") == "4" and show.get("NRestarts") == "0"
                  and alerts == [f"{name}.service"],
                  f"systemd {u}: a reorg stop FAILS the unit with status 4, no restart, one alert ({show}, alerts {alerts})")
        else:
            check(show.get("SubState") == "auto-restart" and show.get("ExecMainStatus") == "1" and not alerts,
                  f"systemd {u}: an ordinary failure (exit 1) still restarts, no alert ({show})")
    finally:
        sc("stop", f"{name}.service")
        sc("reset-failed", f"{name}.service")
        shutil.rmtree(ud / f"{name}.service.d", ignore_errors=True)
        for f in (ud / f"{name}.service", ud / f"{alert}@.service"): f.unlink(missing_ok=True)
        sc("daemon-reload")
        shutil.rmtree(r, ignore_errors=True)

if SYSTEMD:
    print("== 3. the real units under systemctl --user")
    if subprocess.run(["systemctl", "--user", "is-system-running"], capture_output=True, text=True).stdout.strip() not in ("running", "degraded"):
        check(False, "a systemd user session (run with --no-systemd where there is none: then this part did NOT run)")
    else:
        tag = f"hzt{os.getpid()}"
        for u in UNITS:
            systemd_case(u, "reorg", tag)
            if not CONTROL: systemd_case(u, "crash", tag)
else:
    print("== 3. SKIPPED (--no-systemd): the units were NOT run under systemd")

print()
if CONTROL:
    want = [f for f in fails if "RestartPreventExitStatus" in f or "a reorg stop FAILS" in f]
    if fails and len(want) == len(fails) and any("RestartPreventExitStatus" in f for f in fails):
        print(f"CONTROL OK: without exit 4 in RestartPreventExitStatus the units fail the check ({len(fails)} failure(s))")
        sys.exit(0)
    print(f"CONTROL FAILED: {fails}")
    sys.exit(1)
if fails:
    print(f"FAILED {len(fails)} of {len(ran)}: " + "; ".join(fails)); sys.exit(1)
print(f"all {len(ran)} checks passed: a reorg stop fails the unit and alerts; a crash still restarts")
