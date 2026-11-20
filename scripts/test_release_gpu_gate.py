#!/usr/bin/env python3
"""The release GPU gate's pod lifecycle (scripts/release_gpu_gate.py `run`), with a FAKE RunPod and a FAKE pod
transport: no network, no GPU, no money. The real sponsor_bot terminate_confirmed / release_until_clean run
against the fake, so this tests the code that actually releases pods.

The one property that matters most: EVERY pod the gate was given is terminated, on every path -- success, a card
that never boots, a failed prove, a failed harvest, a crash in the gate's own code, the spend cap, SIGTERM, and a
"lost create" (RunPod made the pod although the call failed). And no marker unless everything passed: both
architecture families proved and the staged verifier accepted every receipt and REFUSED a tampered one.

  python3 scripts/test_release_gpu_gate.py            # must PASS
  python3 scripts/test_release_gpu_gate.py --control  # the gate stops terminating pods: MUST FAIL
"""
import argparse
import hashlib
import os
import shutil
import signal
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import release_gpu_gate as g  # noqa: E402
import sponsor_bot  # noqa: E402

CONTROL = "--control" in sys.argv
if CONTROL:
    g._CONTROL_NO_TERMINATE = True
    print("CONTROL: the gate no longer terminates pods -- the checks below MUST fail")
IMAGE = g.canon_image()
BLOCKS = g.gate_blocks()
fails = 0
TEMPS = []


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
    """sponsor_bot.RunPod's surface the gate uses. `boots`/`capacity`/`lost` are sets of GPU-type substrings."""
    def __init__(self, cost=1.0, boots=("A40", "Blackwell"), capacity=("A40", "Blackwell"), lost=()):
        self.key, self.cost, self.boots, self.capacity, self.lost = "fake", cost, boots, capacity, lost
        self.live, self.deployed, self.terminated, self.n, self.lock = {}, [], [], 0, threading.Lock()

    def _match(self, gt, keys):
        return any(k in gt for k in keys)

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
            return [{"id": p["id"], "name": p["name"], "status": p["status"]} for p in self.live.values()]


class FakeTransport:
    """The pod side. `plan[family]` says what the pod does: result string (None = never finishes), and which step
    raises. family = 'A40' or 'Blackwell', from the pod's GPU type."""
    def __init__(self, api, host_sha, plan=None):
        self.api, self.host_sha, self.plan, self.pubkey = api, host_sha, plan or {}, "ssh-ed25519 FAKE"
        self.harvested = []

    def fam(self, port):
        gt = self.api.live.get(f"pod{port - 20000}", {}).get("gpu", "")
        return "Blackwell" if "Blackwell" in gt else "A40"

    def step(self, port, name):
        p = self.plan.get(self.fam(port), {})
        if p.get("raise") == name:
            raise RuntimeError(f"injected failure in {name}")
        if p.get("sigterm") == name:
            os.kill(os.getpid(), signal.SIGTERM)
        return p

    def ready(self, ip, port):
        return True

    def push(self, ip, port, sha):
        self.step(port, "push")

    def start(self, ip, port):
        self.step(port, "start")

    def result(self, ip, port):
        return self.step(port, "result").get("result", "PROVED") or ""

    def harvest(self, ip, port, dest):
        p = self.step(port, "harvest")
        fam = self.fam(port)
        d = os.path.join(dest, "out")
        os.makedirs(d, exist_ok=True)
        gpu = ("NVIDIA A40, 8.6, 550.54, 46068 MiB" if fam == "A40"
               else "NVIDIA RTX PRO 4500 Blackwell Server Edition, 12.0, 575.51, 32623 MiB")
        open(os.path.join(d, "gpu.txt"), "w").write(gpu + "\n")
        open(os.path.join(d, "host_sha256"), "w").write(p.get("pod_sha", self.host_sha) + "\n")
        open(os.path.join(d, "method_id.txt"), "w").write(f"METHOD_ID {IMAGE}\n")
        with open(os.path.join(d, "proves.tsv"), "w") as f:
            for h, *_ in BLOCKS:
                f.write(f"{h}\t0\t12.5\n")
        if (p.get("result", "PROVED") or "") == "PROVED":
            for h, _bh, _l, _a, state in BLOCKS:
                os.makedirs(os.path.join(d, f"p{h}"), exist_ok=True)
                open(os.path.join(d, f"p{h}", f"block_{h}.rcpt"), "w").write(f"GOOD-RECEIPT-{h}-" + "x" * 200)
                shutil.copyfile(os.path.join(g.FIXTURES, state), os.path.join(d, f"p{h}", f"state_{h}.json"))
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


def scenario(name, api_kw=None, plan=None, lax=False, source_sha=None, max_usd=2.0):
    """Run the gate once against a fresh fake; returns (rc, api, dist, transport)."""
    t = tempfile.mkdtemp(prefix="hz-relgate-test-")
    TEMPS.append(t)
    dist = os.path.join(t, "dist")
    os.makedirs(dist)
    open(os.path.join(dist, g.CUDA_ASSET), "wb").write(os.urandom(4096))
    host_sha = g.sha256_file(os.path.join(dist, g.CUDA_ASSET))
    open(os.path.join(dist, g.VERIFIER_ASSET), "w").write(VERIFIER.replace("$LAX", "1" if lax else "0"))
    os.chmod(os.path.join(dist, g.VERIFIER_ASSET), 0o755)
    api = FakeAPI(**(api_kw or {}))
    tr = FakeTransport(api, host_sha, plan)
    clock = Clock()
    a = argparse.Namespace(source=dist, host_sha256=source_sha or host_sha, max_usd=max_usd, max_min=60.0,
                           work=os.path.join(t, "work"), dry_run=False)
    print(f"-- {name}")
    try:
        rc = g.cmd_run(a, api=api, transport=tr, balance=lambda _api: 100.0, sleep=clock.sleep, clock=clock)
    except SystemExit as e:
        rc = f"SystemExit({e})"
    return rc, api, dist, tr, host_sha


def no_pod_left(api):
    return not api.live


def has_marker(dist):
    return os.path.isfile(os.path.join(dist, g.MARKER_LINUX))


old_out = sys.stdout


def quiet(fn, *a, **k):
    """Run a scenario with the gate's own log hidden (it is long); the checks print normally."""
    sys.stdout = open(os.devnull, "w")
    try:
        return fn(*a, **k)
    finally:
        sys.stdout.close()
        sys.stdout = old_out


rc, api, dist, tr, sha = quiet(scenario, "happy path")
print("-- happy path: both cards prove, every receipt verifies")
check(rc == 0, f"exit 0 (got {rc})")
check(len(api.deployed) == 2 and no_pod_left(api), f"two pods rented, none left billing (live: {list(api.live)})")
check(set(api.terminated) >= set(api.deployed), "terminate was called for every pod")
check(len(tr.harvested) == 2, "both pods harvested before termination")
import json  # noqa: E402
m = json.load(open(os.path.join(dist, g.MARKER_LINUX))) if has_marker(dist) else {}
check(has_marker(dist) and g.marker_ok_linux(m, sha, g.sha256_file(os.path.join(dist, g.VERIFIER_ASSET)), IMAGE,
                                             [b[0] for b in BLOCKS]) == [],
      "the marker is written and clears exactly this host (marker_ok_linux)")
check(any("refused" in str(b.get("tamper_control", "")) for r in m.get("runs", []) for b in r.get("blocks", [])),
      "the marker records the tampered-receipt control as refused")

for title, kw in [
    ("the Blackwell pod never gets SSH", dict(api_kw=dict(boots=("A40",)))),
    ("a prove fails on the A40 (segfault)", dict(plan={"A40": {"result": "PROVE_FAILED 200000 rc=139"}})),
    ("harvest raises on the Blackwell pod", dict(plan={"Blackwell": {"raise": "harvest"}})),
    ("the gate's own push step crashes", dict(plan={"A40": {"raise": "push"}})),
    ("the pod script cannot start", dict(plan={"Blackwell": {"raise": "start"}})),
    ("the spend cap: pods that never finish at $1000/hr", dict(api_kw=dict(cost=1000.0),
                                                           plan={"A40": {"result": None}, "Blackwell": {"result": None}})),
    ("SIGTERM to the gate while both pods run", dict(plan={"A40": {"sigterm": "result", "result": None},
                                                           "Blackwell": {"result": None}})),
    ("a lost create: RunPod made the Blackwell pod but never answered", dict(api_kw=dict(lost=("Blackwell",)))),
    ("no Blackwell capacity at all", dict(api_kw=dict(capacity=("A40",)))),
    ("the pod ran a DIFFERENT host than the staged one", dict(plan={"A40": {"pod_sha": "0" * 64}})),
    ("the verifier accepts a tampered receipt (checks nothing)", dict(lax=True)),
]:
    rc, api, dist, tr, sha = quiet(scenario, title, **kw)
    print(f"-- {title}")
    check(rc not in (0, None), f"the gate fails (exit {rc})")
    check(not has_marker(dist), "no marker")
    check(no_pod_left(api), f"no pod left billing (live: {list(api.live)}, terminated: {api.terminated})")

rc, api, dist, tr, sha = quiet(scenario, "wrong sha", source_sha="f" * 64)
print("-- the operator's sha256 is not the staged host's")
check(str(rc).startswith("SystemExit") and api.n == 0, f"refused before renting anything (rc {rc}, pods {api.n})")

for t in TEMPS:
    shutil.rmtree(t, ignore_errors=True)
if CONTROL:
    print(f"CONTROL: {fails} failure(s)")
    sys.exit(0 if fails else 1)
print(f"{fails} failure(s)")
sys.exit(1 if fails else 0)
