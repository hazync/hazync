#!/usr/bin/env python3
"""release_gpu_gate.py: the release's REAL-GPU gate. ⚠ `run` SPENDS MONEY (about $0.40-0.80, capped).

v0.26.1 and v0.27.0 published a Linux CUDA host that segfaulted on its first GPU step: the release box has no GPU,
so every check before publishing read BYTES (ids, imports, sizes) and none of them ran a kernel. This gate runs the
EXACT staged binary (by sha256) on real cards, proves real blocks, verifies the receipts with the EXACT staged
verifier, and writes a marker that scripts/release.sh requires before it publishes.

Commands (scripts/release-gpu-gate.sh is the wrapper; docs/BUILD_AND_RELEASE.md "The real-GPU gate" is the procedure):

  run --source BOX:DIST --host-sha256 <sha>   from the MAINTAINER'S LAPTOP, at the release commit. Refuses while an
        hz-relgate-* pod is listed (--sweep-first), names other pods, checks SECURE stock; rents a pre-Blackwell card
        (A40, else A5000/A6000/L40S) and a Blackwell one (PRO 4500, else PRO 6000) in parallel; arms each pod's own
        dead-man; the release box pushes the staged host; each pod proves the GATE blocks (170, full block 200,000);
        harvest, ALWAYS terminate, reap lost creates, sweep; the staged verifier must accept every receipt and refuse
        a tampered one. Fail-fast; caps on this run's own spend (--max-usd 2) and time (--max-min 60).
        PASS => DIST/.gpu-gate-linux-cuda.json on the release box.
  record-windows --source BOX:DIST --report gate-windows.json --receipt block_170.rcpt --log prove.log --zip <zip>
        After scripts/release-gpu-gate-windows.ps1 ran the STAGED zip's host.exe on a real card: checks the report
        against the staged zip's host.exe, the receipt and the log, refuses a receipt ever used as evidence before,
        verifies it, and writes DIST/.gpu-gate-windows-cuda.json, result ATTESTED (the maintainer's word, bound).
  sweep [--terminate]                          list (and release) every hz-relgate-* pod: after ANY abnormal exit.
  check --dist DIST --image <id> --commit <sha>   what scripts/release.sh runs. No network.

The markers catch mistakes; they are not tamper-proof (anyone who can write dist/ can write one).

Env: RUNPOD_API_KEY_FILE (default ~/.hazync/runpod.key), read in-process by coordinator/sponsor_bot.py, never
printed. The balance read is the one GraphQL call (v2 has none); every pod call is REST v2 through sponsor_bot.
"""
import argparse
import datetime
import getpass
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(REPO, "coordinator"))
import sponsor_bot  # noqa: E402

FIXTURES = os.path.join(REPO, "prover", "testdata", "libzync", "gpu-gate")
POD_SCRIPT = os.path.join(HERE, "release-gpu-gate-pod.sh")
CUDA_ASSET = "hazync-host-x86_64-linux-gnu-cuda"
VERIFIER_ASSET = "hazync-verify-x86_64-linux-gnu"
WIN_ASSET = "hazync-host-x86_64-windows-cuda.zip"
MARKER_LINUX = ".gpu-gate-linux-cuda.json"
MARKER_WIN = ".gpu-gate-windows-cuda.json"
SCHEMA = 1
POD_PREFIX = "hz-relgate-"
# (role, RunPod GPU type ids in preference order). Cheapest card that proves each architecture family: a
# pre-Blackwell card -- the A40 (sm_86, $0.49/hr), else an RTX A5000 / A6000 (sm_86) or an L40S (sm_89) -- and a
# Blackwell sm_120 card -- the PRO 4500 ($0.72/hr), else the PRO 6000 ($2.09/hr, the board's card and the one v0.26.1
# crashed on). Prices: docs/OPERATIONS.md 4.1 (measured 2026-09). Only types with SECURE stock are tried (in_stock).
CARDS = (("pre-blackwell", ("NVIDIA A40", "NVIDIA RTX A5000", "NVIDIA RTX A6000", "NVIDIA L40S")),
         ("blackwell", ("NVIDIA RTX PRO 4500 Blackwell Server Edition",
                        "NVIDIA RTX PRO 6000 Blackwell Server Edition")))
SPEND_MARGIN = 1.15          # on the run's own $/hr x time estimate (billing granularity, a pod's last seconds)
DEADMAN_MARGIN_S = 600       # the pod-side dead-man fires this long after the time cap
STOP_SIGNALS = (signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT)   # all handled like Ctrl-C: release the pods first
HISTORY = ".gpu-gate-history.txt"   # on the release box's dist/: every receipt sha256 a marker ever recorded
LOCAL_HISTORY = os.path.expanduser("~/.hazync/gpu-gate-receipts.txt")
GRAPHQL = "https://api.runpod.io/graphql"
UA = "hazync-release-gpu-gate/1 (+https://github.com/hazync/hazync)"
SSH_OPTS = ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR",
            "-o", "ConnectTimeout=20", "-o", "ServerAliveInterval=30", "-o", "BatchMode=yes"]
BOX_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=20"]     # the release box: its host key IS checked
# test_release_gpu_gate.py only: each turns off one of the two independent ways pods are released (per-pod
# terminate; the sweep + lost-create reaper), so the tests can show each works alone and that both off fails.
_CONTROL_NO_TERMINATE = False
_CONTROL_NO_SWEEP = False


def now_utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def canon_image():
    for line in open(os.path.join(REPO, "reproduce", "LIBZYNC_IDS")):
        p = line.split()
        if len(p) >= 4 and p[0] == "libzync-product" and p[1] == "mainnet":
            return p[3]
    raise SystemExit("reproduce/LIBZYNC_IDS has no libzync-product mainnet line")


def gate_blocks(fixtures=FIXTURES):
    """[(height, hash, leaf, anchor, state)] from GATE, after checking every fixture against SHA256SUMS."""
    out = []
    for line in open(os.path.join(fixtures, "GATE")):
        p = line.split()
        if not p or p[0].startswith("#"):
            continue
        if len(p) != 5 or not p[0].isdigit() or not re.fullmatch(r"[0-9a-f]{64}", p[1]):
            raise SystemExit(f"{fixtures}/GATE: bad line {line.strip()!r}")
        out.append((int(p[0]), p[1], p[2], p[3], p[4]))
    if not out:
        raise SystemExit(f"{fixtures}/GATE lists no blocks")
    want = {}
    for line in open(os.path.join(fixtures, "SHA256SUMS")):
        s, n = line.split()
        want[n] = s
    for b in out:
        for n in b[2:]:
            if want.get(n) != sha256_file(os.path.join(fixtures, n)):
                raise SystemExit(f"fixture {n} does not match {fixtures}/SHA256SUMS")
    return out


def is_blackwell(gpu_name, compute_cap=""):
    try:
        if int(str(compute_cap).split(".")[0]) >= 10:
            return True
    except ValueError:
        pass
    return bool(re.search(r"Blackwell|\bB[12]00\b|\bGB[12]00\b|RTX 50[0-9]0", gpu_name or ""))


# ---------------------------------------------------------------------------------------------------------------
# verification: the STAGED verifier (fetched by sha256), on the maintainer's machine
#
# Since the verifier is built in the hosts' ubuntu:22.04 image (scripts/build-linux-verifier.sh) and check-dist.sh
# refuses any Linux asset needing more than glibc 2.34, the staged verifier runs on a 22.04+ laptop. (Before, it was
# built by the release box's own cargo and needed GLIBC_2.39: v0.27.0's does not start on 22.04.) Measured 20.11.2026
# with the v0.27.0 verifier (in an ubuntu:24.04 container) on a real CPU proof of leaf 170 (image 46666b6f): exit 2,
# SWIFTSYNC_VERIFY_DEMO "FROM THE GIVEN ANCHOR ONLY", `[ok] --expect`; the same receipt with its middle byte flipped,
# a wrong --expect hash or a wrong anchor: exit 1; prove-range's state_170.json equals prep's but for anchor_id.

class LocalVerifier:
    def __init__(self, path):
        self.path = path

    def run(self, anchor, expect, receipt):
        r = subprocess.run([self.path, "--anchor", anchor, "--expect", expect, receipt],
                           capture_output=True, text=True, timeout=600)
        return r.returncode, r.stdout, r.stderr


def verify_receipt(verifier, receipt, anchor, height, block_hash):
    """(ok, verdict line). OK means: exit 2 (DEMO: a valid receipt FROM THE GIVEN ANCHOR), a DEMO verdict, no
    failed check, and the --expect identity check present and passed."""
    if isinstance(verifier, str):
        verifier = LocalVerifier(verifier)
    try:
        rc, out, err = verifier.run(anchor, f"{height}:{block_hash}", receipt)
    except (OSError, subprocess.SubprocessError, RuntimeError) as e:
        return False, f"verifier did not run: {e}"
    lines = out.splitlines()
    verdict = next((x for x in reversed(lines) if x.startswith("SWIFTSYNC_VERIFY_")), "(no verdict)")
    if "GLIBC_" in (err or ""):
        verdict += " -- the staged verifier cannot run on this machine's glibc (it must need <= 2.34: check-dist.sh)"
    ok = (rc == 2 and verdict.startswith("SWIFTSYNC_VERIFY_DEMO:")
          and not any(x.lstrip().startswith("[FAIL]") for x in lines)
          and any(x.lstrip().startswith("[ok] --expect") for x in lines))
    return ok, f"exit {rc}: {verdict}"


def tamper_refused(verifier, receipt, anchor, height, block_hash, workdir):
    """The control: the same receipt with one byte flipped mid-seal must NOT verify. A verifier that accepts it is
    not checking anything, and the gate must not believe its other answers."""
    data = bytearray(open(receipt, "rb").read())
    if len(data) < 64:
        return False, "receipt too small to tamper"
    data[len(data) // 2] ^= 0x01
    bad = os.path.join(workdir, f"tampered_{height}.rcpt")
    open(bad, "wb").write(bytes(data))
    ok, verdict = verify_receipt(verifier, bad, anchor, height, block_hash)
    return (not ok), verdict


def state_matches(got_path, want_path):
    """prep's S(h) and the leaf proof's S(h) agree on every field but anchor_id (a leaf is a base step on S(h-1))."""
    try:
        a, b = json.load(open(got_path)), json.load(open(want_path))
    except (OSError, ValueError) as e:
        return False, str(e)
    diff = sorted(k for k in set(a) | set(b) if k != "anchor_id" and a.get(k) != b.get(k))
    return not diff, ("equal but anchor_id" if not diff else f"differs in {diff}")


# ---------------------------------------------------------------------------------------------------------------
# sources: where the staged files are (the release box over ssh, or a local directory)

class Source:
    def __init__(self, spec):
        self.remote = ":" in spec and not os.path.exists(spec)
        self.box, self.dist = (spec.split(":", 1) if self.remote else (None, os.path.abspath(spec)))

    def path(self, name):
        return f"{self.dist.rstrip('/')}/{name}"

    def sha256(self, name):
        if not self.remote:
            p = self.path(name)
            return sha256_file(p) if os.path.isfile(p) else None
        r = subprocess.run(["ssh", *BOX_OPTS, self.box, "sha256sum " + shlex.quote(self.path(name))],
                           capture_output=True, text=True, timeout=300)
        m = re.match(r"([0-9a-f]{64})\s", r.stdout)
        return m.group(1) if (r.returncode == 0 and m) else None

    def fetch(self, name, dest):
        if not self.remote:
            shutil.copyfile(self.path(name), dest)
        else:
            subprocess.run(["scp", "-q", *BOX_OPTS, f"{self.box}:{self.path(name)}", dest], check=True, timeout=1800)
        os.chmod(dest, 0o755)
        return dest

    def read_text(self, name):
        """A small file from the dist, or None when it is not there."""
        if not self.remote:
            p = self.path(name)
            return open(p).read() if os.path.isfile(p) else None
        r = subprocess.run(["ssh", *BOX_OPTS, self.box, "cat " + shlex.quote(self.path(name))],
                           capture_output=True, text=True, timeout=300)
        return r.stdout if r.returncode == 0 else None

    def append_text(self, name, text):
        if not self.remote:
            with open(self.path(name), "a") as f:
                f.write(text)
        else:
            subprocess.run(["ssh", *BOX_OPTS, self.box, "cat >> " + shlex.quote(self.path(name))], input=text,
                           text=True, check=True, timeout=300)

    def put_marker(self, local, name):
        if not self.remote:
            shutil.copyfile(local, self.path(name))
        else:
            subprocess.run(["scp", "-q", *BOX_OPTS, local, f"{self.box}:{self.path(name)}"], check=True, timeout=300)


# ---------------------------------------------------------------------------------------------------------------
# the pod transport (ssh). The tests replace this with a fake; everything that decides lifecycle is in Gate.

def run_cmd(cmd, timeout, stop=None, input_text=None):
    """subprocess.run that a set `stop` Event (or the timeout) kills: a cap must be able to interrupt a 400 MB push.
    Returns (rc, stdout, stderr); rc is -9 when stopped, -15 on timeout."""
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         stdin=subprocess.PIPE if input_text is not None else subprocess.DEVNULL, text=True)
    end, first = time.monotonic() + timeout, True
    while True:
        try:
            out, err = p.communicate(input=input_text if first else None, timeout=0.5)
            return p.returncode, out, err
        except subprocess.TimeoutExpired:
            first = False
            stopped = stop is not None and stop.is_set()
            if stopped or time.monotonic() > end:
                p.kill()
                out, err = p.communicate()
                return (-9 if stopped else -15), out, err


class SshTransport:
    """`stop` (set by Gate) kills any running step; harvest takes its own deadline and ignores `stop`."""

    def __init__(self, key, source, workdir, log):
        self.key, self.source, self.workdir, self.log = key, source, workdir, log
        self.box_dir, self.stop = None, None

    def _pod(self, ip, port, cmd, timeout=600, stop=True):
        return run_cmd(["ssh", "-i", self.key, *SSH_OPTS, "-p", str(port), f"root@{ip}", cmd], timeout,
                       self.stop if stop else None)

    def _scp(self, ip, port, files, dest, timeout=1800, stop=True):
        rc, _o, err = run_cmd(["scp", "-q", "-i", self.key, *SSH_OPTS, "-P", str(port), *files, f"root@{ip}:{dest}"],
                              timeout, self.stop if stop else None)
        if rc:
            raise RuntimeError(f"scp to the pod failed (rc {rc}): {err.strip()[:200]}")

    def ready(self, ip, port):
        return self._pod(ip, port, "true", timeout=60)[0] == 0

    def setup(self):
        """Before any pod: give the release box this run's throwaway pod key, so IT pushes the 400+ MB host straight
        to each pod (a datacenter link) instead of through the laptop's uplink."""
        if not self.source.remote:
            return
        self.box_dir = f"/tmp/hz-relgate-{os.getpid()}-{int(time.time())}"
        b = self.source.box
        subprocess.run(["ssh", *BOX_OPTS, b, f"umask 077 && mkdir -p {self.box_dir}"], check=True, timeout=120)
        subprocess.run(["scp", "-q", *BOX_OPTS, self.key, f"{b}:{self.box_dir}/key"], check=True, timeout=120)

    def teardown(self):
        if self.box_dir:
            subprocess.run(["ssh", *BOX_OPTS, self.source.box, f"rm -rf {self.box_dir}"], timeout=120)
            self.box_dir = None

    def arm(self, ip, port, seconds):
        """The POD-SIDE dead-man, first thing after SSH: the pod deletes (else stops, else powers off) itself after
        `seconds`, whatever happens to the laptop. scripts/release-gpu-gate-pod.sh --deadman."""
        self._scp(ip, port, [POD_SCRIPT], "/workspace/", timeout=300)
        rc, _o, err = self._pod(ip, port, f"cd /workspace && (setsid nohup bash release-gpu-gate-pod.sh --deadman "
                                          f"{int(seconds)} > deadman.out 2>&1 < /dev/null &)", timeout=120)
        if rc:
            raise RuntimeError(f"could not arm the pod's dead-man: {err.strip()[:200]}")

    def push(self, ip, port, host_sha):
        tmp = os.path.join(self.workdir, f"push-{port}")
        os.makedirs(tmp, exist_ok=True)
        open(os.path.join(tmp, "want_host_sha256"), "w").write(host_sha + "\n")
        rc, _o, err = self._pod(ip, port, "mkdir -p /workspace/gate", timeout=120)
        if rc:
            raise RuntimeError(f"pod mkdir failed: {err.strip()[:200]}")
        self._scp(ip, port, [os.path.join(FIXTURES, n) for n in sorted(os.listdir(FIXTURES))], "/workspace/gate/")
        self._scp(ip, port, [os.path.join(tmp, "want_host_sha256")], "/workspace/", timeout=300)
        if self.source.remote:
            cmd = (f"scp -q -i {self.box_dir}/key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "
                   f"-o LogLevel=ERROR -P {int(port)} {shlex.quote(self.source.path(CUDA_ASSET))} root@{ip}:/workspace/host")
            rc, _o, err = run_cmd(["ssh", *BOX_OPTS, self.source.box, cmd], 3600, self.stop)
            if rc:
                raise RuntimeError(f"the release box could not push the host (rc {rc}): {err.strip()[:200]}")
        else:
            rc, _o, err = run_cmd(["scp", "-q", "-C", "-i", self.key, *SSH_OPTS, "-P", str(port),
                                   self.source.path(CUDA_ASSET), f"root@{ip}:/workspace/host"], 7200, self.stop)
            if rc:
                raise RuntimeError(f"could not push the host (rc {rc}): {err.strip()[:200]}")

    def start(self, ip, port):
        rc, _o, err = self._pod(ip, port, "cd /workspace && (setsid nohup bash release-gpu-gate-pod.sh > run.out 2>&1 "
                                          "< /dev/null & echo $! > script.pid)", timeout=120)
        if rc:
            raise RuntimeError(f"could not start the pod script: {err.strip()[:200]}")

    def result(self, ip, port):
        """RESULT's word; POD_SCRIPT_DIED when the script is gone without writing one; "" while it runs."""
        rc, out, _e = self._pod(ip, port, "cd /workspace && { cat out/RESULT 2>/dev/null || "
                                          "{ kill -0 \"$(cat script.pid 2>/dev/null)\" 2>/dev/null || echo POD_SCRIPT_DIED; }; }",
                                timeout=120)
        return out.strip() if rc == 0 else ""

    def harvest(self, ip, port, dest, deadline_s=900):
        """out/ and run.out into dest; the tarball's sha256 checked against the pod's own. Retries until deadline_s
        (wall clock): on an abort path the pod is billing, so the gate gives this 5 minutes, not hours."""
        end = time.monotonic() + deadline_s
        t = 0
        while time.monotonic() < end:
            t += 1
            left = max(10, int(end - time.monotonic()))
            rc, out, _e = self._pod(ip, port, "cd /workspace && tar czf out.tgz out run.out deadman.out 2>/dev/null; "
                                              "sha256sum out.tgz | cut -c1-64", timeout=min(300, left), stop=False)
            want = out.strip()
            tgz = os.path.join(self.workdir, f"out-{port}.tgz")
            rc2, _o, _e = run_cmd(["scp", "-q", "-i", self.key, *SSH_OPTS, "-P", str(port),
                                   f"root@{ip}:/workspace/out.tgz", tgz], max(10, int(end - time.monotonic())))
            if rc == 0 and rc2 == 0 and re.fullmatch(r"[0-9a-f]{64}", want) and sha256_file(tgz) == want:
                os.makedirs(dest, exist_ok=True)
                subprocess.run(["tar", "xzf", tgz, "-C", dest], check=True, timeout=600)
                return True
            self.log(f"harvest try {t} failed")
            time.sleep(min(10, max(0, end - time.monotonic())))
        return False


def runpod_balance(api):
    """clientBalance: the one GraphQL call (REST v2 has no balance). Needs a User-Agent (Cloudflare 1010)."""
    req = urllib.request.Request(GRAPHQL, method="POST",
                                 data=json.dumps({"query": "query { myself { clientBalance } }"}).encode(),
                                 headers={"Content-Type": "application/json", "User-Agent": UA,
                                          "Authorization": f"Bearer {api.key}"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return float(json.load(r)["data"]["myself"]["clientBalance"])


def live_pods(api):
    return [p for p in api.pods() if (p.get("status") or "") != "TERMINATED"]


def in_stock(api, cards):
    """Each role's GPU types, keeping only those RunPod shows SECURE stock for (catalogue availability not NONE).
    Refuses (SystemExit) before anything is rented if a role has none: renting one card and not the other only
    spends money on a gate that cannot pass."""
    cat = api.gpu_catalog(stock=True)
    avail = {g.get("id"): str(g.get("availability") or "NONE").upper() for g in cat}
    out = []
    for role, types in cards:
        ok = tuple(t for t in types if avail.get(t, "NONE") != "NONE")
        if not ok:
            seen = ", ".join(f"{t}: {avail.get(t, 'unlisted')}" for t in types)
            raise SystemExit(f"no SECURE stock for {role} ({seen}): nothing rented; try later")
        out.append((role, ok))
    return tuple(out)


# ---------------------------------------------------------------------------------------------------------------
# the gate

class Abort(Exception):
    pass


class Gate:
    """Rents, runs, harvests and ALWAYS terminates. api: sponsor_bot.RunPod (or a fake); transport: SshTransport (or a
    fake); balance: a function of api returning the account balance, or None to skip balance reads.

    The spend cap is THIS RUN's spend, estimated from its own pods' $/hr x their time alive (lost creates included)
    x SPEND_MARGIN: the account balance also moves with every other pod on the key (the sponsor bot, a fleet), so its
    delta is reported, never acted on."""

    def __init__(self, api, transport, host_sha, workdir, blocks, cards=CARDS, max_usd=2.0, max_min=60.0,
                 boot_s=900, deploy_tries=3, poll_s=30.0, balance=runpod_balance, sleep=time.sleep,
                 clock=time.monotonic, log=print, harvest_abort_s=300, reap_every_s=120):
        self.api, self.t, self.host_sha, self.workdir, self.blocks = api, transport, host_sha, workdir, blocks
        self.cards, self.max_usd, self.max_s = cards, max_usd, max_min * 60
        self.boot_s, self.deploy_tries, self.poll_s = boot_s, deploy_tries, poll_s
        self.balance, self.sleep, self.clock, self._log = balance, sleep, clock, log
        self.harvest_abort_s, self.reap_every_s = harvest_abort_s, reap_every_s
        self.prefix = f"{POD_PREFIX}{host_sha[:8]}-{os.getpid()}-{int(time.time()) % 100000}-"
        self.stop = threading.Event()
        self.t.stop = self.stop
        self.stop_reason = ""
        self.lock = threading.Lock()
        self.created = []           # every pod id we were given, for the final sweep
        self.live = {}              # pod id -> (t_deploy, $/hr) while it may bill (lost creates too)
        self.spent_est = 0.0        # $ of pods already terminated
        self.runs = []
        self.b0 = None
        self.sweep_clean = False
        self.t_start = self.clock()

    def log(self, msg):
        self._log(f"[{datetime.datetime.now().strftime('%H:%M:%S')}] {msg}")

    def halt(self, why):
        if not self.stop.is_set():
            self.stop_reason = why
            self.log(f"STOPPING: {why}")
            self.stop.set()

    def est_spend(self):
        with self.lock:
            t = self.clock()
            raw = self.spent_est + sum((t - t0) * cph / 3600.0 for t0, cph in self.live.values())
        return raw * SPEND_MARGIN

    def deadman_s(self):
        """What the pod's own dead-man waits: the rest of the time cap plus a margin for the harvest."""
        return max(300, int(self.max_s - (self.clock() - self.t_start)) + DEADMAN_MARGIN_S)

    def _wait(self, seconds):
        if self.stop.wait(0):
            raise Abort(self.stop_reason)
        end = self.clock() + seconds
        while self.clock() < end:
            if self.stop.is_set():
                raise Abort(self.stop_reason)
            self.sleep(min(1.0, max(0.0, end - self.clock())))
        if self.stop.is_set():
            raise Abort(self.stop_reason)

    def _forget(self, pid):
        with self.lock:
            t0, cph = self.live.pop(pid, (self.clock(), 0.0))
            self.spent_est += (self.clock() - t0) * cph / 3600.0

    # -- one card --
    def card(self, role, gpu_types):
        run = {"role": role, "gpu_types": list(gpu_types), "pod_id": None, "result": "NOT_STARTED",
               "harvested": False, "terminated": None, "error": ""}
        self.runs.append(run)
        pod, ip_port = None, None
        try:
            for attempt in range(self.deploy_tries):
                if self.stop.is_set():
                    raise Abort(self.stop_reason)
                try:
                    pod = self.api.deploy(f"{self.prefix}{role}", self.t.pubkey, gpu_types=tuple(gpu_types))
                except sponsor_bot.RunPodError as e:
                    self.log(f"{role}: deploy refused: {e}")
                    pod = None
                if pod and pod.get("id"):
                    break
                self.log(f"{role}: no capacity for {list(gpu_types)} (try {attempt + 1}/{self.deploy_tries})")
                pod = None
                if attempt + 1 < self.deploy_tries:
                    self._wait(60)
            if not pod:
                raise RuntimeError(f"no {role} card could be rented")
            pid = str(pod["id"])
            with self.lock:
                self.created.append(pid)
                self.live[pid] = (self.clock(), float(pod.get("cost_per_hr") or 0.0))
            run.update(pod_id=pid, gpu_type=pod.get("gpu_type"), cost_per_hr=float(pod.get("cost_per_hr") or 0.0))
            self.log(f"{role}: pod {pid} {pod.get('gpu_type')} ${run['cost_per_hr']:.2f}/hr")
            t0 = self.clock()
            while self.clock() - t0 < self.boot_s:
                try:
                    s = self.api.ssh_of(pid)
                except sponsor_bot.RunPodError as e:     # one unanswered GET is not a dead pod; keep waiting
                    self.log(f"{role}: pod lookup failed ({e}); retrying")
                    s = None
                if s and self.t.ready(*s):
                    ip_port = tuple(s)
                    break
                self._wait(15)
            if not ip_port:
                raise RuntimeError(f"{role}: no SSH within {self.boot_s} s")
            run["boot_s"] = round(self.clock() - t0, 1)
            run["deadman_s"] = self.deadman_s()
            self.t.arm(*ip_port, run["deadman_s"])       # before anything long: the laptop may die from here on
            self.log(f"{role}: ssh {ip_port[0]}:{ip_port[1]} after {run['boot_s']} s; dead-man {run['deadman_s']} s; pushing")
            self.t.push(*ip_port, self.host_sha)
            self.t.start(*ip_port)
            run["result"] = "RUNNING"
            t_run = self.clock()
            while True:
                self._wait(self.poll_s)
                res = self.t.result(*ip_port)
                if res:
                    run["result"] = res
                    break
            run["run_s"] = round(self.clock() - t_run, 1)
            self.log(f"{role}: pod says {run['result']} after {run['run_s']} s")
        except Abort as e:
            run["error"] = f"aborted: {e}"
        except Exception as e:          # noqa: BLE001 -- any failure still harvests and terminates below
            run["error"] = f"{type(e).__name__}: {e}"
            self.log(f"{role}: {run['error']}")
        finally:
            if run["result"] != "PROVED":   # FAIL FAST: the gate cannot pass now, so stop the other card's bill too
                self.halt(f"the {role} card ended without PROVED ({run['result']}{'; ' + run['error'] if run['error'] else ''})")
            pid = run["pod_id"]
            if pid:
                if ip_port:         # HARVEST FIRST: a terminated pod's evidence is gone for good -- but bounded
                    try:
                        run["harvested"] = bool(self.t.harvest(*ip_port, os.path.join(self.workdir, role),
                                                               900 if run["result"] == "PROVED" else self.harvest_abort_s))
                    except Exception as e:  # noqa: BLE001
                        run["error"] += f"; harvest: {e}"
                if not _CONTROL_NO_TERMINATE:
                    try:
                        run["terminated"] = bool(sponsor_bot.terminate_confirmed(self.api, pid, sleep=self.sleep))
                    except Exception as e:  # noqa: BLE001 -- the sweep in run() tries again
                        run["terminated"] = False
                        run["error"] += f"; terminate: {e}"
                self._forget(pid)
                self.log(f"{role}: pod {pid} terminated={run['terminated']}")
        return run

    def reap_lost(self):
        """A pod carrying this run's prefix that no deploy call handed back (a lost create) bills from the moment it
        exists: count it in the spend estimate and release it, during the run, not only at the end."""
        if _CONTROL_NO_SWEEP:
            return
        try:
            rows = live_pods(self.api)
        except Exception as e:  # noqa: BLE001
            self.log(f"listing for lost creates failed: {e}")
            return
        with self.lock:
            lost = [p for p in rows if (p.get("name") or "").startswith(self.prefix) and str(p["id"]) not in self.created]
            for p in lost:
                pid = str(p["id"])
                self.created.append(pid)
                self.live.setdefault(pid, (self.clock(), float(p.get("cost_per_hr") or 0.0)))
        for p in lost:
            self.log(f"lost create {p['id']} ({p.get('name')}): releasing it")
            try:
                done, _left = sponsor_bot.terminate_many(self.api, [str(p["id"])], self.sleep)
                for pid in done:
                    self._forget(pid)
            except Exception as e:  # noqa: BLE001 -- the final sweep tries again
                self.log(f"could not release lost create {p['id']}: {e}")

    # -- the whole gate --
    def run(self):
        self.t_start = self.clock()
        threads = [threading.Thread(target=self.card, args=c, name=c[0]) for c in self.cards]
        interrupted = False
        try:
            if self.balance:
                self.b0 = self.balance(self.api)
                self.log(f"balance ${self.b0:.2f}")
                if self.b0 < self.max_usd:
                    raise SystemExit(f"balance ${self.b0:.2f} is below the spend cap ${self.max_usd:.2f}: top up first")
            for th in threads:
                th.start()
            last_bal = last_reap = self.clock()
            while any(th.is_alive() for th in threads):
                for th in threads:
                    th.join(timeout=1.0)
                if self.est_spend() >= self.max_usd:
                    self.halt(f"spend cap: this run's estimated ${self.est_spend():.2f} >= ${self.max_usd:.2f}")
                if self.clock() - self.t_start >= self.max_s:
                    self.halt(f"time cap: {self.max_s / 60:.0f} min")
                if self.clock() - last_reap >= self.reap_every_s:
                    last_reap = self.clock()
                    self.reap_lost()
                if self.balance and self.clock() - last_bal >= 300:
                    last_bal = self.clock()
                    try:
                        b = self.balance(self.api)
                        if self.b0 - b > max(self.est_spend() * 2, 0.5):
                            self.log(f"⚠ the balance fell ${self.b0 - b:.2f}, more than this run's own estimate "
                                     f"${self.est_spend():.2f}: other pods on this key are spending (not acted on)")
                    except Exception as e:  # noqa: BLE001
                        self.log(f"balance read failed: {e}")
        except KeyboardInterrupt:
            interrupted = True
            self.halt("interrupted")
            for th in threads:
                while th.is_alive():
                    try:
                        th.join()
                    except KeyboardInterrupt:
                        self.log("still releasing pods -- wait")
        finally:
            for th in threads:
                while th.is_alive():
                    try:
                        th.join()
                    except KeyboardInterrupt:
                        self.log("still releasing pods -- wait")
            self.sweep()
        if interrupted:
            raise KeyboardInterrupt
        return self.runs

    def sweep(self):
        """THE backstop: every pod this run created, and any pod carrying this run's name prefix (a lost create that
        RunPod made although the call failed), released until two listings show none. Signals that would stop the
        process are ignored while it runs: a second Ctrl-C must not leave pods billing."""
        if _CONTROL_NO_SWEEP:
            self.sweep_clean = not any(r["pod_id"] and r["terminated"] is not True for r in self.runs)
            return
        main = threading.current_thread() is threading.main_thread()
        old = {}
        if main:
            for s in STOP_SIGNALS + (signal.SIGINT,):
                old[s] = signal.signal(s, signal.SIG_IGN)
        try:
            try:
                done, left, clean = sponsor_bot.release_until_clean(
                    self.api, list(self.created), match=lambda n: n.startswith(self.prefix), sleep=self.sleep)
            except Exception as e:  # noqa: BLE001
                done, left, clean = [], list(self.created), False
                self.log(f"SWEEP FAILED: {e}")
        finally:
            for s, h in old.items():
                signal.signal(s, h)
        for pid in done:
            self._forget(pid)
        self.sweep_clean = bool(clean) and not left
        if left or not clean:
            self.log(f"⛔ PODS MAY STILL BE BILLING: {left} -- run: scripts/release-gpu-gate.sh sweep --terminate")
        for r in self.runs:
            if r["pod_id"] and r["pod_id"] in done and r["terminated"] is not True:
                r["terminated"] = True


def judge(gate, runs, verifier, blocks, image, workdir):
    """Verify every harvested receipt with the staged verifier. True only if every card proved and verified."""
    ok, tampered = True, False
    for r in runs:
        d = os.path.join(workdir, r["role"], "out")
        r["blocks"] = []
        if r["result"] != "PROVED" or not r["harvested"] or r["terminated"] is not True:
            ok = False
            continue

        def read(name):
            p = os.path.join(d, name)
            return open(p).read() if os.path.isfile(p) else ""
        # nvidia-smi: "<name>, <compute_cap>, <driver>, <memory>"
        parts = [x.strip() for x in (read("gpu.txt").strip().splitlines() or [""])[0].split(",")]
        r["gpu_name"], r["compute_cap"] = (parts + ["", ""])[:2]
        r["host_sha256_on_pod"] = read("host_sha256").strip()
        r["method_id_ok"] = image in read("method_id.txt")
        if r["host_sha256_on_pod"] != gate.host_sha or not r["method_id_ok"]:
            ok = False
        times = {}
        for line in read("proves.tsv").splitlines():
            p = line.split()
            if len(p) == 3:
                times[int(p[0])] = float(p[2])
        for h, bh, _leaf, anchor, state in blocks:
            rc = os.path.join(d, f"p{h}", f"block_{h}.rcpt")
            a = os.path.join(FIXTURES, anchor)
            b = {"height": h, "block_hash": bh, "prove_s": times.get(h)}
            if not os.path.isfile(rc):
                b.update(verified=False, verdict="no receipt harvested")
                ok = False
            else:
                v, verdict = verify_receipt(verifier, rc, a, h, bh)
                sm, sm_note = state_matches(os.path.join(d, f"p{h}", f"state_{h}.json"), os.path.join(FIXTURES, state))
                b.update(verified=v, verdict=verdict, receipt_sha256=sha256_file(rc), state_matches_prep=sm, state_note=sm_note)
                if v and not tampered:
                    refused, tv = tamper_refused(verifier, rc, a, h, bh, workdir)
                    b["tamper_control"] = ("refused: " if refused else "ACCEPTED: ") + tv
                    tampered = True
                    if not refused:
                        ok = False
                if not (v and sm):
                    ok = False
            r["blocks"].append(b)
            gate.log(f"{r['role']} {r.get('gpu_name')}: block {h}: {b['verdict']}")
    roles = [is_blackwell(r.get("gpu_name"), r.get("compute_cap")) for r in runs if r.get("blocks")]
    if not (True in roles and False in roles):
        ok = False
    if not tampered:
        ok = False
    return ok


def marker_ok_linux(m, host_sha, verifier_sha, image, heights, commit=None, fixtures_sha=None):
    """Reasons a Linux CUDA marker does NOT clear the staged files ([] = it does). A mistake-catcher, not a
    signature: anyone who can write dist/ can write a marker (docs/BUILD_AND_RELEASE.md)."""
    why = []
    if not isinstance(m, dict):
        return ["not a JSON object"]
    if m.get("schema") != SCHEMA or m.get("kind") != "linux-cuda":
        why.append(f"schema/kind {m.get('schema')}/{m.get('kind')!r}, want {SCHEMA}/'linux-cuda'")
    if m.get("result") != "PASS":
        why.append(f"result is {m.get('result')!r}, not PASS")
    if m.get("asset") != CUDA_ASSET:
        why.append(f"asset {m.get('asset')!r}")
    if m.get("host_sha256") != host_sha:
        why.append(f"it is for host {str(m.get('host_sha256'))[:16]}..., the staged host is {str(host_sha)[:16]}...")
    if m.get("verifier_sha256") != verifier_sha:
        why.append(f"receipts were verified by {str(m.get('verifier_sha256'))[:16]}..., the staged verifier is {str(verifier_sha)[:16]}...")
    if m.get("image") != image:
        why.append(f"image {str(m.get('image'))[:8]}, the release ships {image[:8]}")
    if commit and m.get("repo_commit") != commit:
        why.append(f"the gate ran from commit {str(m.get('repo_commit'))[:8]}, the release is {commit[:8]}")
    if fixtures_sha and m.get("fixtures_sha256") != fixtures_sha:
        why.append("the gate proved other fixtures than this tree's prover/testdata/libzync/gpu-gate")
    runs = m.get("runs") if isinstance(m.get("runs"), list) else []
    kinds = set()
    for r in runs:
        if not isinstance(r, dict):
            why.append("a run is not an object")
            continue
        got = {b.get("height") for b in r.get("blocks") or [] if isinstance(b, dict) and b.get("verified") is True
               and re.fullmatch(r"[0-9a-f]{64}", str(b.get("receipt_sha256", "")))}
        if not set(heights) <= got:
            why.append(f"{r.get('gpu_name')!r}: verified blocks {sorted(got)}, the gate needs {sorted(heights)}")
            continue
        if r.get("host_sha256_on_pod") != host_sha or r.get("terminated") is not True:
            why.append(f"{r.get('gpu_name')!r}: host on the pod {str(r.get('host_sha256_on_pod'))[:16]}..., terminated={r.get('terminated')}")
            continue
        kinds.add(is_blackwell(r.get("gpu_name"), r.get("compute_cap")))
    if True not in kinds:
        why.append("no Blackwell card proved and verified every gate block")
    if False not in kinds:
        why.append("no pre-Blackwell card proved and verified every gate block")
    return why


def zip_host_exe_sha(zip_path):
    """sha256 of host.exe inside the Windows zip, or None."""
    import zipfile
    try:
        with zipfile.ZipFile(zip_path) as z:
            names = [n for n in z.namelist() if n.replace("\\", "/").split("/")[-1].lower() == "host.exe"]
            if len(names) != 1:
                return None
            return hashlib.sha256(z.read(names[0])).hexdigest()
    except (OSError, zipfile.BadZipFile):
        return None


def marker_ok_windows(m, zip_sha, verifier_sha, image, host_exe_sha=None, commit=None):
    why = []
    if not isinstance(m, dict):
        return ["not a JSON object"]
    if m.get("schema") != SCHEMA or m.get("kind") != "windows-cuda":
        why.append(f"schema/kind {m.get('schema')}/{m.get('kind')!r}, want {SCHEMA}/'windows-cuda'")
    if m.get("result") != "ATTESTED":
        why.append(f"result is {m.get('result')!r}, not ATTESTED")
    if m.get("asset") != WIN_ASSET:
        why.append(f"asset {m.get('asset')!r}")
    if m.get("zip_sha256") != zip_sha:
        why.append(f"it is for zip {str(m.get('zip_sha256'))[:16]}..., the staged zip is {str(zip_sha)[:16]}...")
    if host_exe_sha is not None and m.get("host_exe_sha256") != host_exe_sha:
        why.append(f"the run hashed host.exe {str(m.get('host_exe_sha256'))[:16]}..., the staged zip's is {str(host_exe_sha)[:16]}...")
    if m.get("verifier_sha256") != verifier_sha:
        why.append("the receipt was verified by a different verifier than the staged one")
    if m.get("image") != image:
        why.append(f"image {str(m.get('image'))[:8]}, the release ships {image[:8]}")
    if commit and m.get("repo_commit") != commit:
        why.append(f"recorded from commit {str(m.get('repo_commit'))[:8]}, the release is {commit[:8]}")
    if not str(m.get("gpu_name") or "").strip():
        why.append("no GPU named")
    blocks = m.get("blocks") if isinstance(m.get("blocks"), list) else []
    if not any(isinstance(b, dict) and b.get("height") == 170 and b.get("verified") is True
               and re.fullmatch(r"[0-9a-f]{64}", str(b.get("receipt_sha256", ""))) for b in blocks):
        why.append("no verified receipt of block 170")
    return why


def cmd_check(a):
    dist = a.dist
    image = a.image or canon_image()
    heights = [b[0] for b in gate_blocks()]
    fsha = sha256_file(os.path.join(FIXTURES, "SHA256SUMS"))
    fails = []

    def sha(n):
        p = os.path.join(dist, n)
        return sha256_file(p) if os.path.isfile(p) else None

    host, ver, z = sha(CUDA_ASSET), sha(VERIFIER_ASSET), sha(WIN_ASSET)
    for n, s in ((CUDA_ASSET, host), (VERIFIER_ASSET, ver), (WIN_ASSET, z)):
        if not s:
            fails.append(f"{n} is not staged in {dist}")

    def load(n):
        p = os.path.join(dist, n)
        if not os.path.isfile(p):
            return None, f"no {n} in {dist}"
        try:
            return json.load(open(p)), None
        except ValueError as e:
            return None, f"{n} is not JSON ({e})"

    m, err = load(MARKER_LINUX)
    if err:
        fails.append(f"Linux CUDA host: {err} -- run the real-GPU gate (scripts/release-gpu-gate.sh run ...)")
    elif host and ver:
        fails += [f"Linux CUDA host: {w}" for w in marker_ok_linux(m, host, ver, image, heights, a.commit, fsha)]
    w, err = load(MARKER_WIN)
    if err:
        fails.append(f"Windows CUDA host: {err} -- prove block 170 on a real card (scripts/release-gpu-gate-windows.ps1) "
                     "and run scripts/release-gpu-gate.sh record-windows")
    elif z and ver:
        hx = zip_host_exe_sha(os.path.join(dist, WIN_ASSET))
        if not hx:
            fails.append(f"Windows CUDA host: {WIN_ASSET} does not hold exactly one host.exe")
        fails += [f"Windows CUDA host: {x}" for x in marker_ok_windows(w, z, ver, image, hx, a.commit)]
    if fails:
        for f in fails:
            print(f"  REFUSED  {f}")
        return 1
    print(f"  ok   real-GPU gate: the staged Linux CUDA host {host[:16]}... PROVED {heights} on "
          + ", ".join(f"{r.get('gpu_name')}" for r in m["runs"]) + f" ({m.get('finished')})")
    print(f"  ok   the staged Windows CUDA host {z[:16]}... is ATTESTED by {w.get('recorded_by')} to have proved block "
          f"170 on {w.get('gpu_name')} ({w.get('recorded_at')}): the maintainer's word, bound to host.exe's sha256")
    return 0


def who():
    try:
        n = subprocess.run(["git", "-C", REPO, "config", "user.name"], capture_output=True, text=True).stdout.strip()
    except OSError:
        n = ""
    return n or getpass.getuser()


def repo_commit():
    """(HEAD of this checkout, dirty?). The marker carries it; release.sh requires it to be the release commit."""
    try:
        c = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        d = bool(subprocess.run(["git", "-C", REPO, "status", "--porcelain"], capture_output=True, text=True).stdout.strip())
    except OSError:
        c, d = "", True
    return c, d


def seen_receipts(src):
    """Every receipt sha256 already used as evidence: the Linux marker in the dist, the dist's history, and this
    machine's own history. A proof is randomised, so the same receipt bytes twice means one was REUSED."""
    seen = set()
    for text in (src.read_text(MARKER_LINUX), src.read_text(HISTORY),
                 open(LOCAL_HISTORY).read() if os.path.isfile(LOCAL_HISTORY) else None):
        if text:
            seen |= set(re.findall(r"\b[0-9a-f]{64}\b", text))
    return seen


def remember(src, shas, what):
    line = "".join(f"{s} {what} {now_utc()}\n" for s in shas)
    os.makedirs(os.path.dirname(LOCAL_HISTORY), exist_ok=True)
    with open(LOCAL_HISTORY, "a") as f:
        f.write(line)
    try:
        src.append_text(HISTORY, line)
    except (subprocess.SubprocessError, OSError) as e:
        print(f"could not append to {HISTORY} at the source: {e}")


def gate_pods(api, prefix=POD_PREFIX):
    return [p for p in live_pods(api) if (p.get("name") or "").startswith(prefix)]


def preflight_pods(api, sweep_first, log=print):
    """Before renting: a leftover gate pod (a crashed laptop, a SIGKILL) is refused or swept; any other pod on the
    key is named loudly (it does not move this run's cap, which counts only this run's pods)."""
    rows = live_pods(api)
    stale = [p for p in rows if (p.get("name") or "").startswith(POD_PREFIX)]
    others = [p for p in rows if not (p.get("name") or "").startswith(POD_PREFIX)]
    if stale:
        names = ", ".join(f"{p['id']} {p.get('name')} ${float(p.get('cost_per_hr') or 0):.2f}/hr" for p in stale)
        if not sweep_first:
            raise SystemExit(f"⛔ gate pods from an earlier run are still billing: {names}\n"
                             "   run: scripts/release-gpu-gate.sh sweep --terminate   (or pass --sweep-first)")
        log(f"sweeping {len(stale)} leftover gate pod(s): {names}")
        _d, left, clean = sponsor_bot.release_until_clean(api, [p["id"] for p in stale],
                                                          match=lambda n: n.startswith(POD_PREFIX))
        if left or not clean:
            raise SystemExit(f"could not release the leftover gate pods {left}: nothing rented")
    if others:
        log(f"⚠ {len(others)} other pod(s) on this RunPod key (not counted in this run's cap): "
            + "; ".join(f"{p['id']} {p.get('name')} ${float(p.get('cost_per_hr') or 0):.2f}/hr" for p in others))
    return others


def cmd_sweep(a, api=None):
    if api is None:
        os.environ.setdefault("RUNPOD_API_KEY_FILE", os.path.expanduser("~/.hazync/runpod.key"))
        api = sponsor_bot.RunPod.from_env()
    pods = gate_pods(api)
    for p in pods:
        print(f"  {p['id']}  {p.get('name')}  {p.get('status')}  ${float(p.get('cost_per_hr') or 0):.2f}/hr")
    if not pods:
        print("no hz-relgate-* pods")
        return 0
    if not a.terminate:
        print(f"{len(pods)} gate pod(s) listed; --terminate releases them")
        return 3
    done, left, clean = sponsor_bot.release_until_clean(api, [p["id"] for p in pods],
                                                        match=lambda n: n.startswith(POD_PREFIX))
    print(f"released {done}; still listed {left}; clean={clean}")
    return 0 if (clean and not left) else 1


def cmd_run(a, api=None, transport=None, balance=runpod_balance, sleep=time.sleep, clock=time.monotonic, **gate_kw):
    host_sha = a.host_sha256.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", host_sha):
        raise SystemExit("--host-sha256 must be 64 hex digits (release.sh prints the staged files' sha256s in step 4)")
    image = canon_image()
    blocks = gate_blocks()
    work = os.path.abspath(a.work or tempfile.mkdtemp(prefix="hz-relgate-"))
    os.makedirs(work, exist_ok=True)
    src = Source(a.source)
    got = src.sha256(CUDA_ASSET)
    if got != host_sha:
        raise SystemExit(f"the staged {CUDA_ASSET} at {a.source} is {got}, not {host_sha}: nothing rented")
    vsha = src.sha256(VERIFIER_ASSET)
    if not vsha:
        raise SystemExit(f"no staged {VERIFIER_ASSET} at {a.source}: nothing rented")
    vpath = src.fetch(VERIFIER_ASSET, os.path.join(work, VERIFIER_ASSET))
    if sha256_file(vpath) != vsha:
        raise SystemExit("the fetched verifier is not the staged one: nothing rented")
    verifier = LocalVerifier(vpath)
    commit, dirty = repo_commit()
    print(f"gate: host {host_sha[:16]}..., verifier {vsha[:16]}..., image {image[:8]}, blocks {[b[0] for b in blocks]}, "
          f"caps ${a.max_usd:.2f} / {a.max_min:.0f} min, commit {commit[:8]}{' (DIRTY)' if dirty else ''}, work {work}")
    if a.dry_run:
        print("DRY RUN: nothing rented")
        return 0
    if api is None:
        os.environ.setdefault("RUNPOD_API_KEY_FILE", os.path.expanduser("~/.hazync/runpod.key"))
        api = sponsor_bot.RunPod.from_env()
    others = preflight_pods(api, a.sweep_first)
    cards = in_stock(api, CARDS)
    print("cards: " + "; ".join(f"{r}: {', '.join(t)}" for r, t in cards))
    if transport is None:
        key = os.path.join(work, "podkey")
        if not os.path.exists(key):
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "hazync-release-gpu-gate", "-f", key],
                           check=True)
        transport = SshTransport(key, src, work, print)
        transport.pubkey = open(key + ".pub").read().strip()
    gate = Gate(api, transport, host_sha, work, blocks, cards=cards, max_usd=a.max_usd, max_min=a.max_min,
                balance=balance, sleep=sleep, clock=clock, **gate_kw)
    started = now_utc()
    rc_interrupt = False

    def _stop(*_):
        raise KeyboardInterrupt
    old = {s: signal.signal(s, _stop) for s in STOP_SIGNALS}
    try:
        if hasattr(transport, "setup"):
            transport.setup()
        runs = gate.run()
    except KeyboardInterrupt:
        runs, rc_interrupt = gate.runs, True
    finally:
        for s, h in old.items():
            signal.signal(s, h)
        if hasattr(transport, "teardown"):
            try:
                transport.teardown()
            except Exception as e:  # noqa: BLE001
                print(f"could not remove the pod key from the release box: {e}")
    spent = None
    if balance and gate.b0 is not None:
        try:
            spent = round(gate.b0 - balance(api), 2)
        except Exception:  # noqa: BLE001
            spent = None
    ok = (not rc_interrupt) and gate.sweep_clean and judge(gate, runs, verifier, blocks, image, work)
    if ok and src.sha256(VERIFIER_ASSET) != vsha:
        print("the staged verifier changed during the gate")
        ok = False
    report = {"schema": SCHEMA, "kind": "linux-cuda", "result": "PASS" if ok else "FAIL", "asset": CUDA_ASSET,
              "host_sha256": host_sha, "verifier_sha256": vsha, "image": image,
              "fixtures_sha256": sha256_file(os.path.join(FIXTURES, "SHA256SUMS")),
              "repo_commit": commit, "repo_dirty": dirty,
              "started": started, "finished": now_utc(), "spent_usd_estimate": round(gate.est_spend(), 2),
              "balance_delta_usd": spent, "other_pods_at_start": len(others), "stop_reason": gate.stop_reason,
              "signed_off_by": who(), "runs": runs}
    rp = os.path.join(work, "gate-report.json")
    json.dump(report, open(rp, "w"), indent=1)
    print(f"report: {rp}")
    if not ok:
        print("GATE FAIL: no marker written. Do NOT publish this CUDA host." + (" (interrupted)" if rc_interrupt else ""))
        if not gate.sweep_clean:
            print("⛔ some pods may still bill: scripts/release-gpu-gate.sh sweep --terminate")
        return 130 if rc_interrupt else 1
    mp = os.path.join(work, MARKER_LINUX)
    json.dump(report, open(mp, "w"), indent=1)
    src.put_marker(mp, MARKER_LINUX)
    remember(src, [b["receipt_sha256"] for r in runs for b in r.get("blocks", []) if b.get("receipt_sha256")],
             f"linux-cuda {host_sha[:16]}")
    print(f"GATE PASS: {MARKER_LINUX} written to {a.source}; scripts/release.sh can publish this exact host.")
    return 0


def cmd_record_windows(a):
    """The Windows CUDA host: the maintainer ran scripts/release-gpu-gate-windows.ps1 with the STAGED zip's host.exe
    on a real card. Its JSON report binds the run to host.exe's sha256; this checks that against the staged zip, the
    receipt and log against the report, refuses a receipt already used as evidence, and verifies it."""
    image = canon_image()
    blocks = {b[0]: b for b in gate_blocks()}
    h, bh, _leaf, anchor, _state = blocks[170]
    src = Source(a.source)
    zsha = src.sha256(WIN_ASSET)
    if not zsha:
        raise SystemExit(f"no staged {WIN_ASSET} at {a.source}")
    if sha256_file(a.zip) != zsha:
        raise SystemExit(f"{a.zip} is not the staged {WIN_ASSET} ({zsha[:16]}...): prove with the STAGED zip")
    hx = zip_host_exe_sha(a.zip)
    if not hx:
        raise SystemExit(f"{a.zip} does not hold exactly one host.exe")
    try:
        rep = json.load(open(a.report))
    except (OSError, ValueError) as e:
        raise SystemExit(f"{a.report}: {e}")
    rsha, lsha = sha256_file(a.receipt), sha256_file(a.log)
    why = []
    if rep.get("host_exe_sha256", "").lower() != hx:
        why.append(f"the run hashed host.exe {str(rep.get('host_exe_sha256'))[:16]}..., the staged zip's is {hx[:16]}...")
    if rep.get("zip_sha256") and rep["zip_sha256"].lower() != zsha:
        why.append("the run names another zip")
    if rep.get("receipt_sha256", "").lower() != rsha:
        why.append(f"{a.receipt} is not the receipt the run wrote")
    if rep.get("log_sha256", "").lower() != lsha:
        why.append(f"{a.log} is not the log the run wrote")
    if rep.get("exit_code") != 0:
        why.append(f"the prove exited {rep.get('exit_code')}")
    if not str(rep.get("gpu_name") or "").strip():
        why.append("the run names no GPU (nvidia-smi)")
    if image not in str(rep.get("method_id") or ""):
        why.append(f"host.exe method-id is not {image[:8]}")
    if rsha in seen_receipts(src):
        why.append("this receipt was already used as evidence (the Linux gate's marker or an earlier record): "
                   "a proof is randomised, so a fresh run never repeats one")
    if why:
        for w in why:
            print(f"  REFUSED  {w}")
        return 1
    vsha = src.sha256(VERIFIER_ASSET)
    if not vsha:
        raise SystemExit(f"no staged {VERIFIER_ASSET} at {a.source}")
    with tempfile.TemporaryDirectory(prefix="hz-relgate-win-") as work:
        return _record_windows(a, src, image, h, bh, anchor, zsha, hx, rep, rsha, lsha, vsha, work)


def _record_windows(a, src, image, h, bh, anchor, zsha, hx, rep, rsha, lsha, vsha, work):
    vpath = src.fetch(VERIFIER_ASSET, os.path.join(work, VERIFIER_ASSET))
    if sha256_file(vpath) != vsha:
        raise SystemExit("the fetched verifier is not the staged one")
    verifier = LocalVerifier(vpath)
    a_path = os.path.join(FIXTURES, anchor)
    ok, verdict = verify_receipt(verifier, a.receipt, a_path, h, bh)
    refused, tv = tamper_refused(verifier, a.receipt, a_path, h, bh, work) if ok else (False, "")
    print(f"block 170 receipt: {verdict}" + (f"; tampered copy: {tv}" if ok else ""))
    if not (ok and refused):
        print("NOT RECORDED: the receipt did not verify (or the verifier accepted a tampered copy).")
        return 1
    commit, dirty = repo_commit()
    m = {"schema": SCHEMA, "kind": "windows-cuda", "result": "ATTESTED", "asset": WIN_ASSET, "zip_sha256": zsha,
         "host_exe_sha256": hx, "verifier_sha256": vsha, "image": image, "gpu_name": rep.get("gpu_name"),
         "compute_cap": rep.get("compute_cap"), "driver": rep.get("driver"), "prove_s": rep.get("prove_s"),
         "blocks": [{"height": 170, "block_hash": bh, "receipt_sha256": rsha, "verified": True,
                     "verdict": verdict, "tamper_control": "refused: " + tv}],
         "prove_log_sha256": lsha, "report": rep, "repo_commit": commit, "repo_dirty": dirty,
         "recorded_by": who(), "recorded_at": now_utc(),
         "attestation": "the maintainer ran scripts/release-gpu-gate-windows.ps1 with the staged zip's host.exe on "
                        "this GPU; the receipt verifies and was never used before, the run is the maintainer's word"}
    mp = os.path.join(work, MARKER_WIN)
    json.dump(m, open(mp, "w"), indent=1)
    src.put_marker(mp, MARKER_WIN)
    remember(src, [rsha], f"windows-cuda {hx[:16]}")
    print(f"RECORDED (ATTESTED): {MARKER_WIN} written to {a.source}")
    return 0


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--source", required=True, help="BOX:DIST (the release box's dist/) or a local dist directory")
    r.add_argument("--host-sha256", required=True)
    r.add_argument("--max-usd", type=float, default=2.0)
    r.add_argument("--max-min", type=float, default=60.0)
    r.add_argument("--work", help="local work directory (default: a new temp dir); evidence is kept there")
    r.add_argument("--dry-run", action="store_true", help="check the inputs and print the plan; rent nothing")
    r.add_argument("--sweep-first", action="store_true", help="release leftover hz-relgate-* pods instead of refusing")
    w = sub.add_parser("record-windows")
    w.add_argument("--source", required=True)
    w.add_argument("--report", required=True, help="the JSON scripts/release-gpu-gate-windows.ps1 wrote")
    w.add_argument("--receipt", required=True, help="block_170.rcpt that run wrote")
    w.add_argument("--log", required=True, help="the prove log that run wrote")
    w.add_argument("--zip", required=True, help="your local copy of the staged zip (checked against the staged sha256)")
    s = sub.add_parser("sweep", help="list (and with --terminate release) every hz-relgate-* pod on the key")
    s.add_argument("--terminate", action="store_true")
    c = sub.add_parser("check")
    c.add_argument("--dist", required=True)
    c.add_argument("--image")
    c.add_argument("--commit", help="the release commit: each marker must have been made from it")
    a = p.parse_args(argv)
    try:
        if a.cmd == "check":
            return cmd_check(a)
        if a.cmd == "record-windows":
            return cmd_record_windows(a)
        if a.cmd == "sweep":
            return cmd_sweep(a)
        return cmd_run(a)
    except sponsor_bot.RunPodError as e:
        print(f"RunPodError: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
