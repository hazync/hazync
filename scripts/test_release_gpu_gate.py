#!/usr/bin/env python3
"""The release GPU gate (scripts/release_gpu_gate.py) with a FAKE RunPod and a FAKE pod transport: no network, no
GPU, no money. The real sponsor_bot terminate_confirmed / terminate_many / release_until_clean run against the fake,
so this tests the code that actually releases pods.

1. Lifecycle. EVERY pod the gate was given is released on every path -- success, a card that never boots, a failed
   prove, a failed harvest, a crash in the gate's own code, a script that dies, the spend cap, SIGTERM, SIGHUP, and
   a "lost create" (RunPod made the pod although the call failed) -- and no marker unless everything passed. Run
   three times: as shipped; with ONLY the per-pod terminate (the sweep and lost-create reaper off); with ONLY the
   sweep (per-pod terminate off). Each mechanism must be enough alone.
2. Gate decisions: fail-fast, the spend cap counting only the run's own pods, leftover gate pods refused (or swept
   with --sweep-first), other pods named but left alone, the SECURE stock check and the pre-Blackwell fallback, the
   dead-man armed on every pod, and judge's refusals (state mismatch, wrong method-id, a missing receipt).
3. record-windows: the run's report must name the staged zip's host.exe, the receipt and log; a receipt already used
   as evidence (the Linux marker, the history) is refused.

  python3 scripts/test_release_gpu_gate.py            # must PASS
  python3 scripts/test_release_gpu_gate.py --control  # both release mechanisms off: MUST FAIL
"""
import argparse
import io
import json
import os
import shutil
import signal
import sys
import tempfile
import threading
import time
import zipfile
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import release_gpu_gate as g  # noqa: E402
import sponsor_bot  # noqa: E402

CONTROL = "--control" in sys.argv
CLEAN = ("c0ffee" * 7)[:40]
g._repo_commit_real = g.repo_commit
g.repo_commit = lambda: (CLEAN, False)        # the tree under test may be dirty; the gate's own dirty check is tested below
BROKEN = {"on": False}


class BrokenOut(io.StringIO):
    """A terminal that hung up: once BROKEN, every write raises EIO, as a print to a closed pty does after SIGHUP."""
    def write(self, x):
        if BROKEN["on"]:
            raise OSError(5, "Input/output error")
        return super().write(x)
IMAGE = g.canon_image()
BLOCKS = g.gate_blocks()
fails = 0
TEMPS = []
A40, A5000 = "NVIDIA A40", "NVIDIA RTX A5000"


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    if not cond:
        fails += 1


class Clock:
    """A shared fake clock: sleep() advances it, so caps and timeouts measured in hours take milliseconds."""
    def __init__(self):
        self.t, self.lock = 1000.0, threading.Lock()

    def __call__(self):
        with self.lock:
            return self.t

    def sleep(self, s):
        with self.lock:
            self.t += max(float(s), 0.0)
        time.sleep(0.0005)


class FakeAPI:
    """sponsor_bot.RunPod's surface the gate uses. `boots`/`capacity`/`lost` are sets of GPU-type substrings;
    `stock` maps a GPU type to its catalogue availability (default LOW); `pre` are pods already on the account."""
    def __init__(self, cost=1.0, boots=("A40", "A5000", "Blackwell"), capacity=("A40", "A5000", "Blackwell"),
                 lost=(), stock=None, pre=()):
        self.key, self.cost, self.boots, self.capacity, self.lost = "fake", cost, boots, capacity, lost
        self.stock = stock or {}
        self.live, self.deployed, self.terminated, self.n, self.lock = {}, [], [], 0, threading.Lock()
        for name in pre:
            self.n += 1
            self.live[f"pod{self.n}"] = {"id": f"pod{self.n}", "name": name, "status": "RUNNING", "gpu": "X"}
        self.pre = list(self.live)

    def _match(self, gt, keys):
        return any(k in gt for k in keys)

    def gpu_catalog(self, stock=False):
        types = {t for _r, ts in g.CARDS for t in ts}
        return [{"id": t, "availability": self.stock.get(t, "LOW")} for t in types]

    def deploy(self, name, pub, gpu_types):
        with self.lock:
            for gt in gpu_types:
                if self._match(gt, self.lost):           # RunPod made it, the answer never came
                    self.n += 1
                    self.live[f"pod{self.n}"] = {"id": f"pod{self.n}", "name": name, "status": "RUNNING", "gpu": gt}
                    raise sponsor_bot.RunPodError("TimeoutError: no answer")
                if self._match(gt, self.capacity):
                    self.n += 1
                    pid = f"pod{self.n}"
                    self.live[pid] = {"id": pid, "name": name, "status": "RUNNING", "gpu": gt}
                    self.deployed.append(pid)
                    return {"id": pid, "name": name, "gpu_type": gt, "cost_per_hr": self.cost}
            return None

    def ssh_of(self, pid):
        p = self.live.get(pid)
        return ("10.9.0.1", 20000 + int(pid[3:])) if p and self._match(p["gpu"], self.boots) else None

    def terminate(self, pid):
        with self.lock:
            self.terminated.append(pid)
            self.live.pop(pid, None)

    def pods(self):
        with self.lock:
            return [{"id": p["id"], "name": p["name"], "status": p["status"], "cost_per_hr": self.cost}
                    for p in self.live.values()]


class FakeTransport:
    """The pod side. `plan[family]` says what the pod does: `result` (None = never finishes), `raise`/`signal` at a
    step, `pod_sha`, and judge-side faults (`state_bad`, `mid_bad`, `no_receipt`). family: 'A40'/'Blackwell'."""
    def __init__(self, api, host_sha, plan=None):
        self.api, self.host_sha, self.plan, self.pubkey = api, host_sha, plan or {}, "ssh-ed25519 FAKE"
        self.harvested, self.armed, self.stop = [], {}, None

    def fam(self, port):
        gt = self.api.live.get(f"pod{port - 20000}", {}).get("gpu", "")
        return "Blackwell" if "Blackwell" in gt else "A40"

    def step(self, port, name):
        p = self.plan.get(self.fam(port), {})
        if p.get("raise") == name:
            raise RuntimeError(f"injected failure in {name}")
        if p.get("signal_at") == name:
            if p.get("break_stdout"):
                BROKEN["on"] = True
            os.kill(os.getpid(), p["signal"])
        return p

    def ready(self, ip, port):
        return True

    def arm(self, ip, port, seconds, pod_id):
        self.armed[port] = (seconds, pod_id)

    def push(self, ip, port, sha):
        self.step(port, "push")

    def start(self, ip, port):
        self.step(port, "start")

    def result(self, ip, port):
        return self.step(port, "result").get("result", "PROVED") or ""

    def harvest(self, ip, port, dest, deadline_s=900):
        p = self.step(port, "harvest")
        fam = self.fam(port)
        d = os.path.join(dest, "out")
        os.makedirs(d, exist_ok=True)
        gpu = ("NVIDIA A40, 8.6, 550.54, 46068 MiB" if fam == "A40"
               else "NVIDIA RTX PRO 4500 Blackwell Server Edition, 12.0, 575.51, 32623 MiB")
        open(os.path.join(d, "gpu.txt"), "w").write(gpu + "\n")
        open(os.path.join(d, "host_sha256"), "w").write(p.get("pod_sha", self.host_sha) + "\n")
        open(os.path.join(d, "method_id.txt"), "w").write(f"METHOD_ID {'0' * 64 if p.get('mid_bad') else IMAGE}\n")
        with open(os.path.join(d, "proves.tsv"), "w") as f:
            for h, *_ in BLOCKS:
                f.write(f"{h}\t0\t12.5\n")
        if (p.get("result", "PROVED") or "") == "PROVED":
            for h, _bh, _l, _a, state in BLOCKS:
                os.makedirs(os.path.join(d, f"p{h}"), exist_ok=True)
                if not p.get("no_receipt"):
                    open(os.path.join(d, f"p{h}", f"block_{h}.rcpt"), "w").write(f"GOOD-RECEIPT-{h}-" + "x" * 200)
                st = json.load(open(os.path.join(g.FIXTURES, state)))
                if p.get("state_bad"):
                    st["utxo_leaves"] = int(st.get("utxo_leaves") or 0) + 1
                json.dump(st, open(os.path.join(d, f"p{h}", f"state_{h}.json"), "w"))
        self.harvested.append(port)
        return True


# A stand-in for the staged verifier: accepts exactly an untouched GOOD receipt for the --expect height (DEMO, exit
# 2, with the --expect line), refuses anything else (exit 1). LAX accepts everything: a verifier that checks nothing.
VERIFIER = r'''#!/bin/bash
r="${@: -1}"; h=""; while [ $# -gt 1 ]; do [ "$1" = --expect ] && h="${2%%:*}"; shift; done
if [ "$LAX" = 1 ] || [ "$(tr -d 'x' < "$r")" = "GOOD-RECEIPT-$h-" ]; then
  echo "  [ok] --expect: block $h is ..."; echo "SWIFTSYNC_VERIFY_DEMO: consistent with the chain FROM THE GIVEN ANCHOR ONLY"; exit 2
fi
echo "  [FAIL] receipt does not verify"; echo "SWIFTSYNC_VERIFY_FAIL: 1 check(s) failed"; exit 1
'''


def mkdist(lax=False):
    t = tempfile.mkdtemp(prefix="hz-relgate-test-")
    TEMPS.append(t)
    dist = os.path.join(t, "dist")
    os.makedirs(dist)
    open(os.path.join(dist, g.CUDA_ASSET), "wb").write(os.urandom(4096))
    open(os.path.join(dist, g.VERIFIER_ASSET), "w").write(VERIFIER.replace("$LAX", "1" if lax else "0"))
    os.chmod(os.path.join(dist, g.VERIFIER_ASSET), 0o755)
    g.LOCAL_HISTORY = os.path.join(t, "local-history.txt")
    return t, dist


def scenario(api_kw=None, plan=None, lax=False, source_sha=None, max_usd=4.0, sweep_first=False, broken=False, work=None):
    """Run the gate once against a fresh fake; returns (rc, api, dist, transport, report)."""
    t, dist = mkdist(lax)
    host_sha = g.sha256_file(os.path.join(dist, g.CUDA_ASSET))
    api = FakeAPI(**(api_kw or {}))
    tr = FakeTransport(api, host_sha, plan)
    clock = Clock()
    a = argparse.Namespace(source=dist, host_sha256=source_sha or host_sha, max_usd=max_usd, max_min=60.0,
                           work=work or os.path.join(t, "work"), dry_run=False, sweep_first=sweep_first)
    BROKEN["on"] = False
    with redirect_stdout(BrokenOut() if broken else io.StringIO()):
        try:
            rc = g.cmd_run(a, api=api, transport=tr, balance=lambda _api: 100.0, sleep=clock.sleep, clock=clock)
        except SystemExit as e:
            rc = f"SystemExit({e})"
        finally:
            BROKEN["on"] = False
    rp = os.path.join(t, "work", "gate-report.json")
    return rc, api, dist, tr, (json.load(open(rp)) if os.path.isfile(rp) else {})


def billing(api):
    return [p for p in api.live if p not in api.pre]


def has_marker(dist):
    return os.path.isfile(os.path.join(dist, g.MARKER_LINUX))


SIGS = (signal.SIGTERM, signal.SIGHUP)
FAILING = [
    ("the Blackwell pod never gets SSH", dict(api_kw=dict(boots=("A40",)))),
    ("a prove fails on the A40 (segfault)", dict(plan={"A40": {"result": "PROVE_FAILED 200000 rc=139"}})),
    ("the pod script dies without a RESULT", dict(plan={"Blackwell": {"result": "POD_SCRIPT_DIED"}})),
    ("harvest raises on the Blackwell pod", dict(plan={"Blackwell": {"raise": "harvest"}})),
    ("the gate's own push step crashes", dict(plan={"A40": {"raise": "push"}})),
    ("the pod script cannot start", dict(plan={"Blackwell": {"raise": "start"}})),
    ("the spend cap: pods that never finish at $1000/hr", dict(api_kw=dict(cost=1000.0),
                                                           plan={"A40": {"result": None}, "Blackwell": {"result": None}})),
    ("SIGTERM to the gate while both pods run", dict(plan={"A40": {"signal_at": "result", "signal": signal.SIGTERM,
                                                                   "result": None}, "Blackwell": {"result": None}})),
    ("SIGHUP (the terminal closed) while both pods run", dict(plan={"A40": {"signal_at": "result", "signal": signal.SIGHUP,
                                                                            "result": None}, "Blackwell": {"result": None}})),
    ("SIGHUP AND the terminal gone (every print raises EIO)", dict(broken=True, plan={
        "A40": {"signal_at": "result", "signal": signal.SIGHUP, "break_stdout": True, "result": None},
        "Blackwell": {"result": None}})),
    ("a lost create: RunPod made the Blackwell pod but never answered", dict(api_kw=dict(lost=("Blackwell",)))),
    ("no Blackwell capacity at all", dict(api_kw=dict(capacity=("A40", "A5000")))),
    ("the pod ran a DIFFERENT host than the staged one", dict(plan={"A40": {"pod_sha": "0" * 64}})),
    ("the verifier accepts a tampered receipt (checks nothing)", dict(lax=True)),
    ("judge: the pod's state differs from prep's", dict(plan={"Blackwell": {"state_bad": True}})),
    ("judge: the host reports another method id", dict(plan={"A40": {"mid_bad": True}})),
    ("judge: PROVED but no receipt was harvested", dict(plan={"A40": {"no_receipt": True}})),
]

# ---- 1. lifecycle, under each release mechanism alone ------------------------------------------------------------
configs = [("as shipped", False, False), ("per-pod terminate ONLY (sweep + reaper off)", False, True),
           ("sweep ONLY (per-pod terminate off)", True, False)]
if CONTROL:
    configs = [("CONTROL: both off", True, True)]
    print("CONTROL: per-pod terminate AND the sweep are off -- the checks below MUST fail")
for cname, no_term, no_sweep in configs:
    g._CONTROL_NO_TERMINATE, g._CONTROL_NO_SWEEP = no_term, no_sweep
    print(f"== lifecycle: {cname}")
    rc, api, dist, tr, rep = scenario()
    sha = g.sha256_file(os.path.join(dist, g.CUDA_ASSET))
    check(rc == 0 and not billing(api) and len(api.deployed) == 2,
          f"happy path: exit 0, two pods, none left billing (rc {rc}, live {billing(api)})")
    m = json.load(open(os.path.join(dist, g.MARKER_LINUX))) if has_marker(dist) else {}
    check(has_marker(dist) and g.marker_ok_linux(m, sha, g.sha256_file(os.path.join(dist, g.VERIFIER_ASSET)), IMAGE,
                                                 [b[0] for b in BLOCKS], m.get("repo_commit"),
                                                 g.sha256_file(os.path.join(g.FIXTURES, "SHA256SUMS"))) == [],
          "  ...the marker clears exactly this host, with the commit and fixtures it ran")
    for title, kw in FAILING:
        if no_sweep and "lost create" in title:
            continue            # a pod no call handed back can ONLY be found by listing: that is the sweep's job
        rc, api, dist, tr, rep = scenario(**kw)
        check(rc not in (0, None) and not has_marker(dist) and not billing(api),
              f"{title}: fails, no marker, no pod left billing (rc {rc}, live {billing(api)})")
g._CONTROL_NO_TERMINATE = g._CONTROL_NO_SWEEP = False
if CONTROL:
    for t in TEMPS:
        shutil.rmtree(t, ignore_errors=True)
    print(f"CONTROL: {fails} failure(s)")
    sys.exit(0 if fails else 1)

# ---- 2. gate decisions --------------------------------------------------------------------------------------------
print("== decisions")
rc, api, dist, tr, rep = scenario()
check(len(tr.armed) == 2 and all(sec >= g.DEADMAN_MARGIN_S and pid in api.deployed for sec, pid in tr.armed.values()),
      f"the pod-side dead-man is armed on every pod with ITS id, cap + margin ({sorted(tr.armed.values())})")
rc, api, dist, tr, rep_ = scenario(broken=True, plan={
    "A40": {"signal_at": "result", "signal": signal.SIGHUP, "break_stdout": True, "result": None},
    "Blackwell": {"result": None}})
check(rc == 130 and len(tr.harvested) == 2 and not billing(api),
      f"SIGHUP with a dead terminal: both pods still harvested and released (rc {rc}, harvested {len(tr.harvested)})")
rc, api, dist, tr, rep = scenario(api_kw=dict(cost=0.0))
check(rc == 0 and rep.get("spent_usd_estimate", 0) > 0 and all(r.get("cost_per_hr", 0) > 0 for r in rep.get("runs", [])),
      f"a pod RunPod prices at $0/hr is counted at its type's ceiling, never free (estimate ${rep.get('spent_usd_estimate')})")
g.repo_commit = lambda: (CLEAN, [" M scripts/oops.py"])
rc, api, dist, tr, rep = scenario()
g.repo_commit = lambda: (CLEAN, False)
check(str(rc).startswith("SystemExit") and api.n == 0 and "scripts/oops.py" in str(rc) and not os.path.isdir(rep.get("_w", "/nonexistent")),
      "a checkout with uncommitted changes: refused before renting, naming the changed files")
rc, api, dist, tr, rep = scenario(work=os.path.join(g.REPO, "relgate-work-test"))
check(str(rc).startswith("SystemExit") and api.n == 0 and "inside the checkout" in str(rc)
      and not os.path.exists(os.path.join(g.REPO, "relgate-work-test")), "--work inside the checkout: refused, nothing created")
rc, api, dist, tr, rep = scenario(max_usd=1.0)
check(str(rc).startswith("SystemExit") and api.n == 0 and "could never hold" in str(rc),
      "a cap below what harvest + terminate can cost at ceiling prices: refused before renting")
_real, _repo = g.__dict__["_repo_commit_real"], g.REPO
g.REPO = tempfile.mkdtemp(prefix="hz-relgate-test-nogit-")
TEMPS.append(g.REPO)
try:
    _real()
    _nogit = "returned"
except SystemExit as e:
    _nogit = f"SystemExit({e})"
g.REPO = _repo
check(_nogit.startswith("SystemExit") and "cannot read this checkout's git state" in _nogit,
      "repo_commit: a git failure is a refusal, never 'clean at commit empty'")
# the reaper must not take a pod whose create call is still in flight
_t, _d = mkdist()
_api = FakeAPI()
_gate = g.Gate(_api, FakeTransport(_api, "0" * 64), "0" * 64, _t, BLOCKS, balance=None, log=lambda m: None)
_name = _gate.prefix + "blackwell"
_api.live["podX"] = {"id": "podX", "name": _name, "status": "RUNNING", "gpu": "Blackwell"}
_gate.inflight.add(_name)
_gate.reap_lost()
check("podX" in _api.live, "the lost-create reaper leaves a pod whose create is still in flight")
_gate.inflight.discard(_name)
_gate.reap_lost()
check("podX" not in _api.live, "  ...and releases it once no call is in flight for its name (a real lost create)")
rc, api, dist, tr, rep = scenario(plan={"A40": {"result": "PROVE_FAILED 170 rc=139"}, "Blackwell": {"result": None}})
check("pre-blackwell card ended without PROVED" in rep.get("stop_reason", ""),
      f"fail-fast: the A40's failure stops the Blackwell card at once (stop_reason: {rep.get('stop_reason')!r})")
rc, api, dist, tr, rep = scenario(api_kw=dict(cost=1000.0), plan={"A40": {"result": None}, "Blackwell": {"result": None}})
check("spend cap: this run's estimated" in rep.get("stop_reason", "") and "still to come" in rep.get("stop_reason", ""),
      f"the spend cap is the run's OWN estimate, minus what harvest + terminate can still cost ({rep.get('stop_reason')!r})")
rc, api, dist, tr, rep = scenario(api_kw=dict(pre=("hz-sponsor-board-1",)))
check(rc == 0 and "pod1" in api.live and rep.get("other_pods_at_start") == 1,
      "another pod on the key is named, left running, and does not stop the gate")
rc, api, dist, tr, rep = scenario(api_kw=dict(pre=("hz-relgate-deadbeef-1-2-blackwell",)))
check(str(rc).startswith("SystemExit") and api.deployed == [] and "pod1" in api.live,
      f"a leftover gate pod is refused before renting anything ({str(rc)[:60]}...)")
rc, api, dist, tr, rep = scenario(api_kw=dict(pre=("hz-relgate-deadbeef-1-2-blackwell",)), sweep_first=True)
check(rc == 0 and "pod1" not in api.live, "  ...and --sweep-first releases it, then runs")
rc, api, dist, tr, rep = scenario(api_kw=dict(stock={t: "NONE" for t in g.CARDS[1][1]}))
check(str(rc).startswith("SystemExit") and api.n == 0, "no SECURE Blackwell stock: refused before renting EITHER card")
rc, api, dist, tr, rep = scenario(api_kw=dict(stock={A40: "NONE"}))
check(rc == 0 and any(r.get("gpu_type") == A5000 for r in rep.get("runs", [])),
      "no A40 stock: the pre-Blackwell card falls back to an RTX A5000")
rc, api, dist, tr, rep = scenario(source_sha="f" * 64)
check(str(rc).startswith("SystemExit") and api.n == 0, "the operator's sha256 is not the staged host's: nothing rented")

ev = threading.Event()
threading.Timer(0.5, ev.set).start()
t0 = time.monotonic()
rc_k = g.run_cmd(["sleep", "30"], 60, ev)[0]
check(rc_k == -9 and time.monotonic() - t0 < 5, "a transport step (run_cmd) is killed when a cap sets stop, not run to its end")

# ---- 3. record-windows ---------------------------------------------------------------------------------------------
print("== record-windows")


def win_case(edit_report=None, reuse=None, zip_other=False, args_drop=None, exe=None, again=False):
    t, dist = mkdist()
    exe = exe or os.urandom(2048)
    zp = os.path.join(dist, g.WIN_ASSET)
    with zipfile.ZipFile(zp, "w") as z:
        z.writestr("host.exe", exe)
        z.writestr("cudart64_12.dll", b"dll")
    local_zip = os.path.join(t, "copy.zip")
    if zip_other:
        with zipfile.ZipFile(local_zip, "w") as z:
            z.writestr("host.exe", b"other")
    else:
        shutil.copyfile(zp, local_zip)
    rc_path, log = os.path.join(t, "block_170.rcpt"), os.path.join(t, "prove.log")
    open(rc_path, "w").write("GOOD-RECEIPT-170-" + "x" * 200)
    open(log, "w").write("proved\n")
    rep = {"host_exe_sha256": g.hashlib.sha256(exe).hexdigest(), "zip_sha256": g.sha256_file(zp),
           "receipt_sha256": g.sha256_file(rc_path), "log_sha256": g.sha256_file(log), "exit_code": 0,
           "gpu_name": "NVIDIA GeForce GTX 1050 Ti", "method_id": f"METHOD_ID {IMAGE}", "prove_s": 197.7}
    if edit_report:
        edit_report(rep)
    rp = os.path.join(t, "gate-windows.json")
    json.dump([rep] if rep.get("_list") else rep, open(rp, "w"))
    if reuse == "linux":
        json.dump({"runs": [{"blocks": [{"receipt_sha256": g.sha256_file(rc_path)}]}]},
                  open(os.path.join(dist, g.MARKER_LINUX), "w"))
    if reuse == "history":
        open(os.path.join(dist, g.HISTORY), "w").write(f"{g.sha256_file(rc_path)} windows-cuda {'9' * 64} old\n")
    if reuse == "same-exe":
        open(os.path.join(dist, g.HISTORY), "w").write(
            f"{g.sha256_file(rc_path)} windows-cuda {g.hashlib.sha256(exe).hexdigest()} old\n")
    argv = ["record-windows", "--source", dist, "--report", rp, "--receipt", rc_path, "--log", log, "--zip", local_zip]
    if args_drop:
        i = argv.index(args_drop)
        del argv[i:i + 2]
    with redirect_stdout(io.StringIO()) as out, open(os.devnull, "w") as dn:
        old = sys.stderr
        sys.stderr = dn
        try:
            rc = g.main(argv)
        except SystemExit as e:
            rc = f"SystemExit({e})"
        finally:
            sys.stderr = old
    return rc, dist, out.getvalue()


rc, dist, out = win_case()
mw = json.load(open(os.path.join(dist, g.MARKER_WIN))) if os.path.isfile(os.path.join(dist, g.MARKER_WIN)) else {}
zp = os.path.join(dist, g.WIN_ASSET)
check(rc == 0 and mw.get("result") == "ATTESTED" and g.marker_ok_windows(
    mw, g.sha256_file(zp), g.sha256_file(os.path.join(dist, g.VERIFIER_ASSET)), IMAGE, g.zip_host_exe_sha(zp)) == [],
      "a fresh run of the staged zip's host.exe is recorded as ATTESTED and clears the staged zip")
check(mw and g.marker_ok_windows(mw, g.sha256_file(zp), g.sha256_file(os.path.join(dist, g.VERIFIER_ASSET)), IMAGE,
                                 "0" * 64) != [], "  ...and check refuses it against a zip whose host.exe differs")
for title, kw in [
    ("the same receipt, already recorded for ANOTHER host.exe", dict(reuse="history")),
    ("report fields that are not strings", dict(edit_report=lambda r: r.update(host_exe_sha256=7, receipt_sha256=None,
                                                                            gpu_name=["x"], method_id={"a": 1}))),
    ("a report that is not a JSON object", dict(edit_report=lambda r: (r.clear(), r.update(_list=True)))),
    ("the Linux gate's own receipt", dict(reuse="linux")),
    ("a report for another host.exe", dict(edit_report=lambda r: r.update(host_exe_sha256="0" * 64))),
    ("a report naming another receipt", dict(edit_report=lambda r: r.update(receipt_sha256="0" * 64))),
    ("a report naming another log", dict(edit_report=lambda r: r.update(log_sha256="0" * 64))),
    ("a prove that exited 1", dict(edit_report=lambda r: r.update(exit_code=1))),
    ("a host.exe of another image", dict(edit_report=lambda r: r.update(method_id="METHOD_ID " + "1" * 64))),
    ("a local zip that is not the staged one", dict(zip_other=True)),
    ("no --log", dict(args_drop="--log")),
    ("no --zip", dict(args_drop="--zip")),
]:
    rc, dist, out = win_case(**kw)
    check(rc != 0 and not os.path.isfile(os.path.join(dist, g.MARKER_WIN)), f"refused: {title} (rc {str(rc)[:40]})")

rc, dist, out = win_case(reuse="same-exe")
check(rc == 0, f"re-recording the same run of the SAME host.exe is allowed (rc {rc})")
_orig = g.Source.read_text
g.Source.read_text = lambda self, name: (_ for _ in ()).throw(RuntimeError("ssh: connection refused"))
rc, dist, out = win_case()
g.Source.read_text = _orig
check(rc == 1 and "could not read the evidence history" in out, "the history cannot be read (ssh down): REFUSED, not taken as empty")
g.repo_commit = lambda: (CLEAN, True)
rc, dist, out = win_case()
g.repo_commit = lambda: (CLEAN, False)
check(rc == 1 and "uncommitted changes" in out, "record-windows from a checkout with uncommitted changes: REFUSED")
g.repo_commit = lambda: (CLEAN, [" M prover/x.rs"])
rc, dist, out = win_case()
g.repo_commit = lambda: (CLEAN, False)
check(rc == 1 and "prover/x.rs" in out, "  ...and the refusal names the changed files")

for t in TEMPS:
    shutil.rmtree(t, ignore_errors=True)
print(f"{fails} failure(s)")
sys.exit(1 if fails else 0)
