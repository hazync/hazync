#!/usr/bin/env python3
"""tip_engine.py (ROADMAP 3.1): the engine tip path, driven end to end on a SIMULATED pod. No network, no money.

  1. next_bundle: the LOWEST unproven height from --tip-from, and a later bundle never jumps an earlier one (the
     968,984 / 968,986 gap);
  2. the per-block script, RUN on a simulated pod (a local directory, a fake product host that proves only with the
     engine's environment): it loads engine.env, sets HAZYNC_ENGINE_GPUS, proves, checks on the pod, prints TIPDONE;
  3. the kill script stops hzengine and the driver host, carried as base64 (the pattern never in the ssh argv);
  4. a whole live_run against a fake RunPod and a fake ssh runner: blocks proved IN ORDER with a gap in the
     bundles' arrival, every proof checked HERE under the pinned key (a wrong pin refuses it), the ledger written,
     the evidence HARVESTED BEFORE THE RELEASE, and the pod released by the run's prefix;
  5. a SIGUSR1 in the middle of the run still harvests and releases (it once killed tip_smoke with pods billing);
     an unknown price refuses to run and still releases; no capacity rents nothing.

  python3 test_tip_engine.py            # must PASS
  python3 test_tip_engine.py --control  # highest-wins selection, no engine.env, no harvest: MUST FAIL
"""
import argparse
import base64
import hashlib
import os
import shutil
import signal
import stat
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import tip_engine  # noqa: E402

fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


FAKE_HOST = r"""#!/bin/bash
# the product host, faked: proves only on the engine (its environment), verifies on the pod (engine env) or here (key+pin)
case "$1" in
  prove-range-bridge)
    [ -n "$HAZYNC_ENGINE" ] && [ -n "$HAZYNC_ENGINE_KEYS" ] && [ -n "$HAZYNC_ENGINE_GPUS" ] || { echo "not on the engine"; exit 3; }
    h=$2; [ -f "leaf_$h.bin" ] || { echo "no leaf_$h.bin"; exit 4; }
    { echo "HZE1 gpus=$HAZYNC_ENGINE_GPUS h=$h"; cat "leaf_$h.bin"; } > "$HAZYNC_BRIDGE_OUT/range_$h.bin"; exit 0;;
  verify-any)
    f=$2; h=$3
    # review R3-6 #4: like the real host on an engine board, no proof is checked without the pinned key
    [ -n "$HAZYNC_ENGINE_VK" ] && [ -n "$HAZYNC_ENGINE_VK_PIN" ] || { echo "REJECT no HAZYNC_ENGINE_VK / _PIN"; exit 5; }
    [ "$(sha256sum < "$HAZYNC_ENGINE_VK" | cut -c1-64)" = "$HAZYNC_ENGINE_VK_PIN" ] || { echo "not the pinned key"; exit 5; }
    grep -q "^HZE1 gpus=[0-9]* h=$h$" "$f" && echo "RANGE-OK lo=$h hi=$h fake" || { echo "bad proof"; exit 6; };;
  *) exit 2;;
esac
"""


def write_exec(path, body):
    with open(path, "w") as f:
        f.write(body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


POD_KEY = b"the release key"
POD_PIN = hashlib.sha256(POD_KEY).hexdigest()


def make_pod(root):
    """A simulated pod's /workspace: the fake host, engine.env as the boot writes it (with the pinned key), a keys dir."""
    os.makedirs(os.path.join(root, "keys-core"), exist_ok=True)
    with open(os.path.join(root, "keys-core", "vk.bin"), "wb") as f:
        f.write(POD_KEY)
    write_exec(os.path.join(root, "hazync-host"), FAKE_HOST)
    with open(os.path.join(root, "engine.env"), "w") as f:
        f.write(f"export HAZYNC_ENGINE={root}/hzengine HAZYNC_ENGINE_KEYS={root}/keys-core\n"
                f"export HAZYNC_ENGINE_VK={root}/keys-core/vk.bin HAZYNC_ENGINE_VK_PIN={POD_PIN}\n")


class FakePod:
    def __init__(self, info):
        self.id, self.name, self.ssh = info["id"], info["name"], None
        self.cost_per_hr = float(info.get("cost_per_hr") or 0)


class FakeRunner:
    """ssh / scp onto a local directory standing in for the pod. ⛔ The kill script is RECORDED, never run: run here
    it would kill real processes on this machine."""
    def __init__(self, root, events, boot_hook=None):
        self.root, self.events, self.boot_hook, self.ssh_pubkey = root, events, boot_hook, "ssh-ed25519 AAAA test"
        self.vk_pin = POD_PIN                 # what SshRunner.prepare() pins from the verified release
        self.killed = []

    def _opts(self):
        return []

    def _ssh(self, pod, cmd, timeout=60):
        if "base64 -d > /tmp/hzkill.sh" in cmd:
            blob = cmd.split()[1]
            self.killed.append(base64.b64decode(blob).decode())
            self.events.append("kill")
            return subprocess.CompletedProcess([], 0, "KILLED\n", "")
        if "tip-harvest.tgz" in cmd:
            self.events.append("harvest")
        r = subprocess.run(["bash", "-c", cmd], capture_output=True, text=True, timeout=timeout)
        if "setsid -f" in cmd:                       # wait for the detached block so the test is deterministic
            subprocess.run(["bash", "-c", "sleep 0.2; while pgrep -f 'tip-[0-9]*.sh' >/dev/null; do sleep 0.1; done"],
                           timeout=60)
        return r

    def _scp(self, pod, paths, dest):
        for p in paths:
            shutil.copy(p, dest)
        return subprocess.CompletedProcess([], 0, "", "")

    def _run(self, argv, timeout):
        if argv[0] == "scp":
            src, dest = argv[-2].split(":", 1)[1], argv[-1]
            if not os.path.isfile(src):
                return subprocess.CompletedProcess(argv, 1, "", "no such file")
            shutil.copy(src, dest)
            return subprocess.CompletedProcess(argv, 0, "", "")
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout)

    def boot(self, pod):
        self.events.append("boot")
        if self.boot_hook:
            self.boot_hook()
        return True, "SHA_OK KEYS_OK CUDART_OK GPU_OK MID ok"


class FakeApi:
    def __init__(self, events, price=6.76, capacity=True):
        self.events, self.price, self.capacity, self.live, self.deploys = events, price, capacity, {}, []

    def deploy(self, name, pubkey, gpu_types=(), gpu_count=1, disk_gb=40):
        self.deploys.append({"name": name, "gpu_types": gpu_types, "gpu_count": gpu_count})
        if not self.capacity:
            return None
        self.live["pod-1"] = name
        return {"id": "pod-1", "name": name, "gpu_type": gpu_types[0], "cost_per_hr": self.price}

    def pod(self, pod_id):
        return {"cost_per_hr": self.price}

    def ssh_of(self, pod_id):
        return ("127.0.0.1", 22)

    def pods(self):
        return [{"id": i, "name": n, "status": "RUNNING"} for i, n in self.live.items()]

    def terminate(self, pod_id):
        self.events.append("terminate")
        self.live.pop(pod_id, None)


def args(bundles, **kw):
    a = argparse.Namespace(gpus=4, gpu_type=list(tip_engine.GPU_TYPES), budget_usd=15.0, minutes=60, tip_from=101,
                           max_blocks=3, bundles=bundles, host=None)
    for k, v in kw.items():
        setattr(a, k, v)
    return a


def main(argv=None):
    global fails
    fails = 0
    argv = sys.argv[1:] if argv is None else argv
    control = "--control" in argv
    orig = (tip_engine.next_bundle, tip_engine.block_script, tip_engine.harvest)
    if control:
        print("CONTROL: highest-wins selection, a block script without engine.env, no harvest -- MUST FAIL")
        tip_engine.next_bundle = lambda avail, proven, tip_from: max([h for h in avail if h not in proven], default=None)
        tip_engine.block_script = lambda h, g, w="/workspace", timeout_s=1800: orig[1](h, g, w, timeout_s).replace(
            ". ./engine.env\n", "")
        tip_engine.harvest = lambda *a, **k: None
    try:
        run_checks()
    finally:
        tip_engine.next_bundle, tip_engine.block_script, tip_engine.harvest = orig
        for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGUSR1, signal.SIGUSR2):
            signal.signal(s, signal.SIG_DFL if s != signal.SIGINT else signal.default_int_handler)
    print(f"{fails} failure(s)")
    return 1 if fails else 0


def run_checks():
    nb = tip_engine.next_bundle
    # 1. selection
    check(nb({101, 102}, set(), 101) == 101, "the lowest unproven height first")
    check(nb({102, 103}, set(), 101) is None, "⛔ a later bundle does not jump a missing earlier one (wait for 101)")
    check(nb({101, 102, 103}, {101}, 101) == 102, "then the next one")
    check(nb({105}, {101, 102, 103, 104}, 101) == 105, "proven heights are passed over")

    tmp = tempfile.mkdtemp(prefix="tipeng-")
    try:
        # 2. the per-block script on a simulated pod
        pod = os.path.join(tmp, "pod1")
        make_pod(pod)
        with open(os.path.join(pod, "leaf_101.bin"), "wb") as f:
            f.write(b"bundle101\n")
        with open(os.path.join(pod, "tip-101.sh"), "w") as f:
            f.write(tip_engine.block_script(101, 4, pod))
        r = subprocess.run(["bash", os.path.join(pod, "tip-101.sh")], capture_output=True, text=True, timeout=60)
        d = tip_engine.parse_tipdone(r.stdout, 101)
        check(d is not None and d["ok"] and d["rc"] == 0 and d["verify"] == 0,
              f"the block script proves on the engine and checks on the pod ({r.stdout.strip()[-120:]})")
        rng = os.path.join(pod, "tip", "101", "range_101.bin")
        check(os.path.isfile(rng) and open(rng).readline().startswith("HZE1 gpus=4 "),
              "every GPU of the pod is used (HAZYNC_ENGINE_GPUS = the pod's count)")
        check(tip_engine.parse_tipdone("TIPDONE 102 rc=0 verify=0 ok=1 prove_ms=1 verify_ms=1", 101) is None,
              "another block's TIPDONE is not this one's")

        # review R3-6 #4: a pod whose engine.env carries no pinned key never passes a block (the old boot wrote none)
        podn = os.path.join(tmp, "pod_nokey")
        make_pod(podn)
        with open(os.path.join(podn, "engine.env"), "w") as f:
            f.write(f"export HAZYNC_ENGINE={podn}/hzengine HAZYNC_ENGINE_KEYS={podn}/keys-core\n")
        with open(os.path.join(podn, "leaf_101.bin"), "wb") as f:
            f.write(b"bundle101\n")
        with open(os.path.join(podn, "tip-101.sh"), "w") as f:
            f.write(tip_engine.block_script(101, 4, podn))
        rn = subprocess.run(["bash", os.path.join(podn, "tip-101.sh")], capture_output=True, text=True, timeout=60)
        dn = tip_engine.parse_tipdone(rn.stdout, 101)
        check(dn is not None and not dn["ok"] and dn["verify"] != 0,
              "⛔ without HAZYNC_ENGINE_VK / _PIN on the pod the per-block check refuses (no unpinned fallback)")
        # and a runner whose pinned release key is not the local pin rents nothing
        rr = FakeRunner(pod, [])
        rr.vk_pin = "00" * 32
        try:
            tip_engine.live_run(args(os.path.join(tmp, "nob")), FakeApi([]), rr, os.path.join(tmp, "run_pin"),
                                os.path.join(tmp, "vk.bin"), POD_PIN, clock=lambda: 0.0, sleep=lambda s: None,
                                workdir=pod)
            check(False, "⛔ a release key that is not HAZYNC_ENGINE_VK_PIN rents nothing")
        except SystemExit as e:
            check("nothing was rented" in str(e), "⛔ a release key that is not HAZYNC_ENGINE_VK_PIN rents nothing")

        # 3. the kill script
        ks = tip_engine.kill_script(True)
        body = base64.b64decode(ks.split()[1]).decode()
        check(all(n in body for n in ("hzengine", "hazync-host")) and "hzengine" not in ks and "hazync-host" not in ks,
              "the kill stops hzengine and the driver host, and neither name is in the ssh argv")

        # 4. a whole run on fakes: 101 and 103 arrive first, 102 later
        events = []
        pod = os.path.join(tmp, "pod2")
        make_pod(pod)
        bundles = os.path.join(tmp, "bundles")
        os.makedirs(bundles)
        for h in (101, 103):
            with open(os.path.join(bundles, f"leaf_{h}.bin"), "wb") as f:
                f.write(f"bundle{h}\n".encode())
        vk = os.path.join(tmp, "vk.bin")
        with open(vk, "wb") as f:
            f.write(b"the release key")
        pin = hashlib.sha256(b"the release key").hexdigest()
        t = [1000.0]
        late = {"done": False}

        def sleep(s):
            t[0] += s
            if t[0] > 1030 and not late["done"]:           # 102 lands after the run has been waiting for it
                with open(os.path.join(bundles, "leaf_102.bin"), "wb") as f:
                    f.write(b"bundle102\n")
                late["done"] = True

        runner = FakeRunner(pod, events)
        api = FakeApi(events)
        rundir = os.path.join(tmp, "run")
        os.makedirs(rundir)
        a = args(bundles, host=os.path.join(pod, "hazync-host"))
        rc = tip_engine.live_run(a, api, runner, rundir, vk, pin, clock=lambda: t[0], sleep=sleep, workdir=pod)
        ledger = open(os.path.join(rundir, tip_engine.tip_stage.LEDGER)).read() if os.path.isfile(
            os.path.join(rundir, tip_engine.tip_stage.LEDGER)) else ""
        order = [int(x.split('"height": ')[1].split(",")[0]) for x in ledger.splitlines() if '"leg": "prove"' in x]
        check(rc == 0, "the run proved its blocks and each passed the pinned check here")
        check(order == [101, 102, 103], f"⛔ blocks proved in height order, 102 waited for, not skipped (got {order})")
        check(api.deploys and api.deploys[0]["gpu_count"] == 4 and api.deploys[0]["name"].startswith(tip_engine.PREFIX),
              "ONE pod of the asked GPU count, named with the run's prefix")
        check('"leg": "verify"' in ledger and '"leg": "push"' in ledger and '"leg": "boot"' in ledger,
              "the ledger has every leg: boot, push, prove, verify")
        check("harvest" in events and "terminate" in events and events.index("harvest") < events.index("terminate"),
              f"⛔ the evidence is harvested BEFORE the pod is released ({events})")
        check(os.path.isfile(os.path.join(rundir, "tip-harvest.tgz")), "the harvest tarball is here")
        check(not api.live, "the pod is gone at the end")

        # a wrong pin: the pinned check here refuses, even though the pod said OK
        ok, detail = tip_engine.local_verify(os.path.join(pod, "hazync-host"), os.path.join(rundir, "range_101.bin"),
                                             101, vk, "00" * 32)
        check(not ok, f"⛔ a proof is refused here under a pin that is not the key's ({detail[-40:]})")
        ok, _ = tip_engine.local_verify(os.path.join(pod, "hazync-host"), os.path.join(rundir, "range_101.bin"),
                                        101, None, None)
        check(not ok, "⛔ no key and pin is a refusal, never a pass")

        # 5. a signal mid-run: still harvested and released
        events2 = []
        pod3 = os.path.join(tmp, "pod3")
        make_pod(pod3)
        runner2 = FakeRunner(pod3, events2, boot_hook=lambda: os.kill(os.getpid(), signal.SIGUSR1))
        api2 = FakeApi(events2)
        rundir2 = os.path.join(tmp, "run2")
        os.makedirs(rundir2)
        raised = None
        try:
            tip_engine.live_run(args(bundles), api2, runner2, rundir2, vk, pin, clock=lambda: t[0],
                                sleep=lambda s: None, workdir=pod3)
        except tip_engine.TeardownSignal as e:
            raised = e
        check(raised is not None, "a SIGUSR1 becomes a teardown, not a silent death")
        check("harvest" in events2 and "terminate" in events2 and events2.index("harvest") < events2.index("terminate")
              and not api2.live, f"⛔ and the pod is still harvested, then released ({events2})")

        # an unknown price: refuse, release
        events3 = []
        api3 = FakeApi(events3, price=0)
        rundir3 = os.path.join(tmp, "run3")
        os.makedirs(rundir3)
        try:
            tip_engine.live_run(args(bundles), api3, FakeRunner(pod3, events3), rundir3, vk, pin, clock=lambda: t[0],
                                sleep=lambda s: None, workdir=pod3)
            refused = False
        except SystemExit as e:
            refused = "no price" in str(e)
        check(refused and not api3.live, "⛔ an unknown price refuses to run unbounded, and the pod is released")

        # no capacity: nothing rented
        api4 = FakeApi([], capacity=False)
        rundir4 = os.path.join(tmp, "run4")
        os.makedirs(rundir4)
        rc4 = tip_engine.live_run(args(bundles), api4, FakeRunner(pod3, []), rundir4, vk, pin, clock=lambda: t[0],
                                  sleep=lambda s: None, workdir=pod3)
        check(rc4 == 0 and not api4.deploys[0].get("rented") and not api4.live, "no capacity of the asked shape: nothing rented, nothing spent (exit 0, as fold_rent)")

        # the plan's quote comes from measured numbers only
        check(abs(tip_engine.cost_quote(6) - 6.76 * (15 * 60 + 6 * 38.6) / 3600) < 1e-9,
              "the quote is the measured price x (15 min setup + the measured 38.6 s per block)")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
