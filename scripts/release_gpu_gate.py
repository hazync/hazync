#!/usr/bin/env python3
"""release_gpu_gate.py: the release's REAL-GPU gate. ⚠ `run` SPENDS MONEY (about $0.40-0.80, capped).

v0.26.1 and v0.27.0 published a Linux CUDA host that segfaulted on its first GPU step: the release box has no GPU,
so every check before publishing read BYTES (ids, imports, sizes) and none of them ran a kernel. This gate runs the
EXACT staged binary (by sha256) on real cards, proves real blocks, verifies the receipts with the EXACT staged
verifier, and writes a marker that scripts/release.sh requires before it publishes.

Commands (scripts/release-gpu-gate.sh is the wrapper; docs/BUILD_AND_RELEASE.md "The real-GPU gate"):

  run --source BOX:DIST --host-sha256 <sha>   from the MAINTAINER'S LAPTOP (the RunPod key never leaves it).
        Rents one pre-Blackwell card (A40) and one Blackwell card (RTX PRO 4500, else RTX PRO 6000), SECURE cloud,
        in parallel; the release box pushes the staged CUDA host to each pod; each pod proves every block of
        prover/testdata/libzync/gpu-gate/GATE (block 170 and full block 200,000) from its real-state leaf bundle;
        the laptop harvests, ALWAYS terminates (confirmed by listing, then a sweep by the run's name prefix), and
        verifies each receipt with the staged verifier, fetched by sha256 (--anchor S(h-1) --expect h:<hash> must
        give DEMO, exit 2), plus one tampered receipt that must be REFUSED. All PASS => DIST/.gpu-gate-linux-cuda.json on the release box.
        Caps: --max-usd (default 2.00; estimated from each pod's $/hr, and the balance) and --max-min (60).
        --source may also be a local dist directory (the laptop then uploads the host itself: slow).
  record-windows --source BOX:DIST --receipt block_170.rcpt --gpu "<name>" [--zip <local copy>] [--log <file>]
        The Windows CUDA host cannot be proved in CI (no GPU). After proving block 170 with the STAGED zip's
        host.exe on a real card (the documented check), this verifies that receipt with the staged verifier and
        writes DIST/.gpu-gate-windows-cuda.json, bound to the staged zip's sha256.
  check --dist DIST --image <id>               what scripts/release.sh runs: exit 0 only if both markers exist and
        name exactly the staged files' sha256s, the published image, PASS, a Blackwell AND a pre-Blackwell card,
        and every gate block verified. No network.

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
# (role, RunPod GPU type ids in preference order). Cheapest card that proves each architecture family: the A40
# (sm_86, $0.49/hr) and a Blackwell sm_120 card -- the PRO 4500 ($0.72/hr), else the PRO 6000 ($2.09/hr, the
# board's card and the one v0.26.1 crashed on). Prices: docs/OPERATIONS.md 4.1 (measured 2026-09).
CARDS = (("pre-blackwell", ("NVIDIA A40",)),
         ("blackwell", ("NVIDIA RTX PRO 4500 Blackwell Server Edition",
                        "NVIDIA RTX PRO 6000 Blackwell Server Edition")))
GRAPHQL = "https://api.runpod.io/graphql"
UA = "hazync-release-gpu-gate/1 (+https://github.com/hazync/hazync)"
SSH_OPTS = ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR",
            "-o", "ConnectTimeout=20", "-o", "ServerAliveInterval=30", "-o", "BatchMode=yes"]
BOX_OPTS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=20"]     # the release box: its host key IS checked
_CONTROL_NO_TERMINATE = False   # test_release_gpu_gate.py --control only: the bug the lifecycle tests must catch


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

    def put_marker(self, local, name):
        if not self.remote:
            shutil.copyfile(local, self.path(name))
        else:
            subprocess.run(["scp", "-q", *BOX_OPTS, local, f"{self.box}:{self.path(name)}"], check=True, timeout=300)


# ---------------------------------------------------------------------------------------------------------------
# the pod transport (ssh). The tests replace this with a fake; everything that decides lifecycle is in Gate.

class SshTransport:
    def __init__(self, key, source, workdir, log):
        self.key, self.source, self.workdir, self.log = key, source, workdir, log
        self.box_dir = None

    def _ssh(self, ip, port, cmd, timeout=600):
        return subprocess.run(["ssh", "-i", self.key, *SSH_OPTS, "-p", str(port), f"root@{ip}", cmd],
                              capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)

    def ready(self, ip, port):
        try:
            return self._ssh(ip, port, "true", timeout=60).returncode == 0
        except subprocess.SubprocessError:
            return False

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

    def push(self, ip, port, host_sha):
        tmp = os.path.join(self.workdir, f"push-{port}")
        os.makedirs(tmp, exist_ok=True)
        open(os.path.join(tmp, "want_host_sha256"), "w").write(host_sha + "\n")
        r = self._ssh(ip, port, "mkdir -p /workspace/gate", timeout=120)
        if r.returncode:
            raise RuntimeError(f"pod mkdir failed: {r.stderr.strip()[:200]}")
        files = [os.path.join(FIXTURES, n) for n in sorted(os.listdir(FIXTURES))]
        subprocess.run(["scp", "-q", "-i", self.key, *SSH_OPTS, "-P", str(port), *files, f"root@{ip}:/workspace/gate/"],
                       check=True, timeout=1800)
        subprocess.run(["scp", "-q", "-i", self.key, *SSH_OPTS, "-P", str(port), POD_SCRIPT,
                        os.path.join(tmp, "want_host_sha256"), f"root@{ip}:/workspace/"], check=True, timeout=600)
        if self.source.remote:
            cmd = (f"scp -q -i {self.box_dir}/key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "
                   f"-o LogLevel=ERROR -P {int(port)} {shlex.quote(self.source.path(CUDA_ASSET))} root@{ip}:/workspace/host")
            subprocess.run(["ssh", *BOX_OPTS, self.source.box, cmd], check=True, timeout=3600)
        else:
            subprocess.run(["scp", "-q", "-C", "-i", self.key, *SSH_OPTS, "-P", str(port), self.source.path(CUDA_ASSET),
                            f"root@{ip}:/workspace/host"], check=True, timeout=7200)

    def start(self, ip, port):
        r = self._ssh(ip, port, "cd /workspace && (setsid nohup bash release-gpu-gate-pod.sh > run.out 2>&1 < /dev/null &)",
                      timeout=120)
        if r.returncode:
            raise RuntimeError(f"could not start the pod script: {r.stderr.strip()[:200]}")

    def result(self, ip, port):
        try:
            r = self._ssh(ip, port, "cat /workspace/out/RESULT 2>/dev/null", timeout=120)
        except subprocess.SubprocessError:
            return ""
        return r.stdout.strip() if r.returncode == 0 else ""

    def harvest(self, ip, port, dest):
        """out/ and run.out into dest; the tarball's sha256 checked against the pod's own. Four tries."""
        for t in range(4):
            try:
                r = self._ssh(ip, port, "cd /workspace && tar czf out.tgz out run.out 2>/dev/null; sha256sum out.tgz | cut -c1-64",
                              timeout=600)
                want = r.stdout.strip()
                tgz = os.path.join(self.workdir, f"out-{port}.tgz")
                subprocess.run(["scp", "-q", "-i", self.key, *SSH_OPTS, "-P", str(port), f"root@{ip}:/workspace/out.tgz", tgz],
                               check=True, timeout=1200)
                if re.fullmatch(r"[0-9a-f]{64}", want) and sha256_file(tgz) == want:
                    os.makedirs(dest, exist_ok=True)
                    subprocess.run(["tar", "xzf", tgz, "-C", dest], check=True, timeout=600)
                    return True
                self.log(f"harvest try {t + 1}: sha256 mismatch")
            except (subprocess.SubprocessError, OSError) as e:
                self.log(f"harvest try {t + 1} failed: {e}")
            time.sleep(10)
        return False


def runpod_balance(api):
    """clientBalance: the one GraphQL call (REST v2 has no balance). Needs a User-Agent (Cloudflare 1010)."""
    req = urllib.request.Request(GRAPHQL, method="POST",
                                 data=json.dumps({"query": "query { myself { clientBalance } }"}).encode(),
                                 headers={"Content-Type": "application/json", "User-Agent": UA,
                                          "Authorization": f"Bearer {api.key}"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return float(json.load(r)["data"]["myself"]["clientBalance"])


# ---------------------------------------------------------------------------------------------------------------
# the gate

class Abort(Exception):
    pass


class Gate:
    """Rents, runs, harvests and ALWAYS terminates. api: sponsor_bot.RunPod (or a fake); transport: SshTransport (or a
    fake); balance: a function of api returning the account balance, or None to skip balance reads."""

    def __init__(self, api, transport, host_sha, workdir, blocks, cards=CARDS, max_usd=2.0, max_min=60.0,
                 boot_s=900, deploy_tries=3, poll_s=30.0, balance=runpod_balance, sleep=time.sleep,
                 clock=time.monotonic, log=print):
        self.api, self.t, self.host_sha, self.workdir, self.blocks = api, transport, host_sha, workdir, blocks
        self.cards, self.max_usd, self.max_s = cards, max_usd, max_min * 60
        self.boot_s, self.deploy_tries, self.poll_s = boot_s, deploy_tries, poll_s
        self.balance, self.sleep, self.clock, self._log = balance, sleep, clock, log
        self.prefix = f"{POD_PREFIX}{host_sha[:8]}-{os.getpid()}-{int(time.time()) % 100000}-"
        self.stop = threading.Event()
        self.stop_reason = ""
        self.lock = threading.Lock()
        self.created = []           # every pod id we were given, for the final sweep
        self.live = {}              # pod id -> (t_deploy, $/hr) while it may bill
        self.spent_est = 0.0        # $ of pods already terminated
        self.runs = []
        self.b0 = None
        self.sweep_clean = False

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
            return self.spent_est + sum((t - t0) * cph / 3600.0 for t0, cph in self.live.values())

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
                name = f"{self.prefix}{role}"
                try:
                    pod = self.api.deploy(name, self.t.pubkey, gpu_types=tuple(gpu_types))
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
            self.log(f"{role}: ssh {ip_port[0]}:{ip_port[1]} after {run['boot_s']} s; pushing inputs")
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
            pid = run["pod_id"]
            if pid:
                if ip_port:         # HARVEST FIRST: a terminated pod's evidence is gone for good
                    try:
                        run["harvested"] = bool(self.t.harvest(*ip_port, os.path.join(self.workdir, role)))
                    except Exception as e:  # noqa: BLE001
                        run["error"] += f"; harvest: {e}"
                if not _CONTROL_NO_TERMINATE:
                    try:
                        run["terminated"] = bool(sponsor_bot.terminate_confirmed(self.api, pid, sleep=self.sleep))
                    except Exception as e:  # noqa: BLE001 -- the sweep in run() tries again
                        run["terminated"] = False
                        run["error"] += f"; terminate: {e}"
                with self.lock:
                    t0, cph = self.live.pop(pid, (self.clock(), 0.0))
                    self.spent_est += (self.clock() - t0) * cph / 3600.0
                self.log(f"{role}: pod {pid} terminated={run['terminated']}")
        return run

    # -- the whole gate --
    def run(self):
        t_start = self.clock()
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
            last_bal = self.clock()
            while any(th.is_alive() for th in threads):
                for th in threads:
                    th.join(timeout=1.0)
                if self.est_spend() >= self.max_usd:
                    self.halt(f"spend cap: estimated ${self.est_spend():.2f} >= ${self.max_usd:.2f}")
                if self.clock() - t_start >= self.max_s:
                    self.halt(f"time cap: {self.max_s / 60:.0f} min")
                if self.balance and self.clock() - last_bal >= 300:
                    last_bal = self.clock()
                    try:
                        b = self.balance(self.api)
                        if self.b0 - b >= self.max_usd:
                            self.halt(f"spend cap: balance fell ${self.b0 - b:.2f}")
                    except Exception as e:  # noqa: BLE001 -- the estimate still caps
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
                if th.is_alive():
                    th.join()
            self.sweep()
        if interrupted:
            raise KeyboardInterrupt
        return self.runs

    def sweep(self):
        """THE backstop: every pod this run created, and any pod carrying this run's name prefix (a lost create that
        RunPod made although the call failed), released until two listings show none."""
        if _CONTROL_NO_TERMINATE:
            return
        try:
            done, left, clean = sponsor_bot.release_until_clean(
                self.api, list(self.created), match=lambda n: n.startswith(self.prefix), sleep=self.sleep)
        except Exception as e:  # noqa: BLE001
            done, left, clean = [], list(self.created), False
            self.log(f"SWEEP FAILED: {e}")
        self.sweep_clean = bool(clean) and not left
        if left or not clean:
            self.log(f"⛔ PODS MAY STILL BE BILLING: {left} -- terminate them by hand (prefix {self.prefix})")
        for r in self.runs:
            if r["pod_id"] and r["pod_id"] in done and r["terminated"] is not True:
                r["terminated"] = True


def judge(gate, runs, verifier, blocks, image, workdir):
    """Verify every harvested receipt with the staged verifier. (all ok, the run records with block results)."""
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
        mid = read("method_id.txt")
        r["method_id_ok"] = image in mid
        if r["host_sha256_on_pod"] != gate.host_sha or not r["method_id_ok"]:
            ok = False
        times = {}
        if os.path.isfile(os.path.join(d, "proves.tsv")):
            for line in open(os.path.join(d, "proves.tsv")):
                p = line.split()
                if len(p) == 3:
                    times[int(p[0])] = float(p[2])
        for h, bh, _leaf, anchor, state in blocks:
            rc = os.path.join(d, f"p{h}", f"block_{h}.rcpt")
            a = os.path.join(FIXTURES, anchor)
            b = {"height": h, "block_hash": bh, "prove_s": times.get(h)}
            if not os.path.isfile(rc):
                b.update(verified=False, verdict="no receipt harvested")
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


def marker_ok_linux(m, host_sha, verifier_sha, image, heights):
    """Reasons a Linux CUDA marker does NOT clear the staged files ([] = it does)."""
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


def marker_ok_windows(m, zip_sha, verifier_sha, image):
    why = []
    if not isinstance(m, dict):
        return ["not a JSON object"]
    if m.get("schema") != SCHEMA or m.get("kind") != "windows-cuda":
        why.append(f"schema/kind {m.get('schema')}/{m.get('kind')!r}, want {SCHEMA}/'windows-cuda'")
    if m.get("result") != "PASS":
        why.append(f"result is {m.get('result')!r}, not PASS")
    if m.get("asset") != WIN_ASSET:
        why.append(f"asset {m.get('asset')!r}")
    if m.get("zip_sha256") != zip_sha:
        why.append(f"it is for zip {str(m.get('zip_sha256'))[:16]}..., the staged zip is {str(zip_sha)[:16]}...")
    if m.get("verifier_sha256") != verifier_sha:
        why.append("the receipt was verified by a different verifier than the staged one")
    if m.get("image") != image:
        why.append(f"image {str(m.get('image'))[:8]}, the release ships {image[:8]}")
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
        fails += [f"Linux CUDA host: {w}" for w in marker_ok_linux(m, host, ver, image, heights)]
    w, err = load(MARKER_WIN)
    if err:
        fails.append(f"Windows CUDA host: {err} -- prove block 170 on a real card and run scripts/release-gpu-gate.sh record-windows")
    elif z and ver:
        fails += [f"Windows CUDA host: {x}" for x in marker_ok_windows(w, z, ver, image)]
    if fails:
        for f in fails:
            print(f"  REFUSED  {f}")
        return 1
    print(f"  ok   real-GPU gate: the staged Linux CUDA host {host[:16]}... proved {heights} on "
          + ", ".join(f"{r.get('gpu_name')}" for r in m["runs"]) + f" ({m.get('finished')}); "
          + f"the staged Windows CUDA host {z[:16]}... proved 170 on {w.get('gpu_name')} ({w.get('recorded_at')})")
    return 0


def who():
    try:
        n = subprocess.run(["git", "-C", REPO, "config", "user.name"], capture_output=True, text=True).stdout.strip()
    except OSError:
        n = ""
    return n or getpass.getuser()


def cmd_run(a, api=None, transport=None, balance=runpod_balance, sleep=time.sleep, clock=time.monotonic):
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
    print(f"gate: host {host_sha[:16]}..., verifier {vsha[:16]}..., image {image[:8]}, blocks {[b[0] for b in blocks]}, "
          f"cards {[c[1][0] for c in CARDS]}, caps ${a.max_usd:.2f} / {a.max_min:.0f} min, work {work}")
    if a.dry_run:
        print("DRY RUN: nothing rented")
        return 0
    if api is None:
        os.environ.setdefault("RUNPOD_API_KEY_FILE", os.path.expanduser("~/.hazync/runpod.key"))
        api = sponsor_bot.RunPod.from_env()
        known = api.gpu_type_ids()
        for role, types in CARDS:
            if not set(types) & known:
                raise SystemExit(f"RunPod lists none of {types} for {role}: nothing rented")
    if transport is None:
        key = os.path.join(work, "podkey")
        if not os.path.exists(key):
            subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "hazync-release-gpu-gate", "-f", key],
                           check=True)
        transport = SshTransport(key, src, work, print)
        transport.pubkey = open(key + ".pub").read().strip()
    gate = Gate(api, transport, host_sha, work, blocks, max_usd=a.max_usd, max_min=a.max_min, balance=balance,
                sleep=sleep, clock=clock)
    started = now_utc()
    rc_interrupt = False
    if hasattr(transport, "setup"):
        transport.setup()
    def _sigterm(*_):
        raise KeyboardInterrupt
    old = signal.signal(signal.SIGTERM, _sigterm)
    try:
        runs = gate.run()
    except KeyboardInterrupt:
        runs, rc_interrupt = gate.runs, True
    finally:
        signal.signal(signal.SIGTERM, old)
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
              "started": started, "finished": now_utc(), "spent_usd_balance": spent,
              "spent_usd_estimate": round(gate.est_spend(), 2), "stop_reason": gate.stop_reason,
              "signed_off_by": who(), "runs": runs}
    rp = os.path.join(work, "gate-report.json")
    json.dump(report, open(rp, "w"), indent=1)
    print(f"report: {rp}")
    if not ok:
        print("GATE FAIL: no marker written. Do NOT publish this CUDA host." + (" (interrupted)" if rc_interrupt else ""))
        return 130 if rc_interrupt else 1
    mp = os.path.join(work, MARKER_LINUX)
    json.dump(report, open(mp, "w"), indent=1)
    src.put_marker(mp, MARKER_LINUX)
    print(f"GATE PASS: {MARKER_LINUX} written to {a.source}; scripts/release.sh can publish this exact host.")
    return 0


def cmd_record_windows(a):
    image = canon_image()
    blocks = {b[0]: b for b in gate_blocks()}
    if 170 not in blocks:
        raise SystemExit("the gate fixtures have no block 170")
    h, bh, _leaf, anchor, _state = blocks[170]
    src = Source(a.source)
    zsha = src.sha256(WIN_ASSET)
    if not zsha:
        raise SystemExit(f"no staged {WIN_ASSET} at {a.source}")
    if a.zip and sha256_file(a.zip) != zsha:
        raise SystemExit(f"{a.zip} is not the staged {WIN_ASSET} ({zsha[:16]}...): prove with the STAGED zip")
    vsha = src.sha256(VERIFIER_ASSET)
    if not vsha:
        raise SystemExit(f"no staged {VERIFIER_ASSET} at {a.source}")
    work = tempfile.mkdtemp(prefix="hz-relgate-win-")
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
    m = {"schema": SCHEMA, "kind": "windows-cuda", "result": "PASS", "asset": WIN_ASSET, "zip_sha256": zsha,
         "verifier_sha256": vsha, "image": image, "gpu_name": a.gpu,
         "blocks": [{"height": 170, "block_hash": bh, "receipt_sha256": sha256_file(a.receipt), "verified": True,
                     "verdict": verdict, "tamper_control": "refused: " + tv}],
         "prove_log_sha256": sha256_file(a.log) if a.log else None,
         "recorded_by": who(), "recorded_at": now_utc(),
         "attestation": "the maintainer ran host.exe from the staged zip on this GPU (docs/BUILD_AND_RELEASE.md)"}
    mp = os.path.join(work, MARKER_WIN)
    json.dump(m, open(mp, "w"), indent=1)
    src.put_marker(mp, MARKER_WIN)
    print(f"RECORDED: {MARKER_WIN} written to {a.source}")
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
    w = sub.add_parser("record-windows")
    w.add_argument("--source", required=True)
    w.add_argument("--receipt", required=True, help="block_170.rcpt proved on Windows by the staged host.exe")
    w.add_argument("--gpu", required=True, help='the card, as nvidia-smi names it, e.g. "NVIDIA GeForce GTX 1050 Ti"')
    w.add_argument("--zip", help="your local copy of the staged zip (checked against the staged sha256)")
    w.add_argument("--log", help="the prove log (its sha256 is recorded)")
    c = sub.add_parser("check")
    c.add_argument("--dist", required=True)
    c.add_argument("--image")
    a = p.parse_args(argv)
    try:
        if a.cmd == "check":
            return cmd_check(a)
        if a.cmd == "record-windows":
            return cmd_record_windows(a)
        return cmd_run(a)
    except sponsor_bot.RunPodError as e:
        print(f"RunPodError: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
