#!/usr/bin/env python3
"""Tip blocks on the engine (OpenVM): ONE rented multi-GPU pod proves each new block as it arrives. (ROADMAP 3.1)

⛔ WHY THIS IS NOT tip_smoke / tip_runner. The RISC Zero tip path spreads ONE block's segments over a fleet of
single-GPU cards (seg-serve on an aggregator, seg-connect on every worker, chunks moved between pods). The engine does
not need that: the product host's engine backend (prover/host/src/engine_backend.rs) runs every chunk, the trie and
the aggregate on the GPUs of ONE machine (HAZYNC_ENGINE_GPUS x provers per GPU), and that measured 33.9 s for block
969,122 on 4x RTX PRO 6000 Blackwell Server Edition (engine/results/gpu_blackwell_4x_969122, 26.11.2026) against
RISC Zero's 191 s on the same card count. So the engine tip path is one pod, booted exactly as the sponsor bot boots an
engine pod (HAZYNC_SPONSOR_ENGINE=1: the signed release's CPU product host + hzengine CUDA + the core key tarball, the
CUDA 12.9 runtime, engine.env, SHA_OK / KEYS_OK / CUDART_OK / GPU_OK and the engine image id), then fed one leaf
bundle per block. tip_smoke / tip_runner / tip_fleet stay the RISC Zero path, unchanged.

Kept from the RISC Zero tip path, each for a measured reason:
  * ⛔ NEVER SKIP A BLOCK (tip_selector, 968,984 + 968,986 left unproven: highest-wins skipped one). next_bundle()
    proves the LOWEST unproven height from --tip-from, and waits for it rather than proving a later one.
  * ⛔ HARVEST BEFORE RELEASE (tips destroyed their own evidence: ten days blaming "nobody ran it"). The finally
    fetches every block's logs and proof file, with retries and a sha check, BEFORE the release.
  * ⛔ RELEASE BY THE RUN'S UNIQUE NAME PREFIX (sponsor_bot.release_until_clean): a pod whose create answer was lost
    is released too, however late it appears. Protected pods are never touched (tip_session.is_protected).
  * ⛔ A SIGNAL IS A TEARDOWN, NOT A DEATH (SIGUSR1 killed tip_smoke with its pods billing). SIGTERM / SIGINT / SIGHUP
    / SIGUSR1 / SIGUSR2 all raise into the finally.
  * ⛔ THE GPU COUNT IS WHAT WAS RENTED, NOT A FLOOR TO REACH (tip_run --cards was a target: six launches, ~$45,
    zero blocks). If no listing of the asked shape has capacity, nothing is rented.
  * ⛔ AN UNKNOWN PRICE IS NOT A FREE POD (fold_rent.seconds_for_budget): no price, no run.
  * ⛔ THE PROOF IS CHECKED HERE, UNDER THE PINNED KEY, not only on the pod: the harvested range file is verified
    with the release's key file and its SHA-256 (HAZYNC_ENGINE_VK / HAZYNC_ENGINE_VK_PIN, engine_backend.rs).

    tip_engine.py plan                      # the measurement plan and its quoted cost; spends nothing
    tip_engine.py selftest                  # the decisions and the teardown paths, no network
    tip_engine.py run --live --budget-usd 15 --minutes 60 --tip-from <h> --bundles <dir>

--bundles is a directory where leaf_<h>.bin files appear (the engine prep / bridge's output, synced from the prep box).
Producing those bundles at the tip is the prep box's job (ROADMAP 3.3); this runner only proves and times them.
"""
import argparse
import base64
import os
import re
import secrets
import signal
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import fold_rent                        # noqa: E402  (seconds_for_budget: the one budget rule)
import sponsor_bot as sb                # noqa: E402  (RunPod client, SshRunner boot, release_until_clean)
import tip_session                      # noqa: E402  (protected pods)
import tip_stage                        # noqa: E402  (the per-leg ledger)

PREFIX = "hz-tip-eng-"
# The measured reference configuration. ⚠ The RunPod type id must be checked against the live catalogue before a run;
# --gpu-type overrides. One type per entry, tried in order; the count is fixed (--gpus).
GPU_TYPES = ("NVIDIA RTX PRO 6000 Blackwell Server Edition",)
GPU_COUNT = 4
DISK_GB = 80                            # the release binaries, keys (~35 MB), bundles and proofs; 40 is the bot's
POLL_S = 5
BLOCK_TIMEOUT_S = 1800                  # one block on the pod; 976,379 took 38.6 s on 4x PRO 6000, 183 s on 1x A6000
HARVEST_TRIES = 3

# Measured, with their sources (only these may be quoted):
MEASURED = {
    "price_4x_pro6000_se_usd_hr": 6.76,       # pod tz427i5mke6391, secure, 26.11.2026 (gpu_blackwell_4x_969122)
    "prove_969122_s": 33.9,                   # 4x PRO 6000 SE, prove-block, same run
    "prove_976379_s": 38.6,                   # same run (segpar)
    "prove_130000_s": 7.2,                    # same run (10 inputs)
}


def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def run_prefix(now=None):
    """<PREFIX><base36 seconds><4 random hex>-: unique per run, so the release sweep takes only this run's pods."""
    t, n = int(now if now is not None else time.time()), ""
    while t:
        t, r = divmod(t, 36)
        n = "0123456789abcdefghijklmnopqrstuvwxyz"[r] + n
    return f"{PREFIX}{n}{secrets.token_hex(2)}-"


def next_bundle(available, proven, tip_from):
    """The next height to prove: the LOWEST height >= tip_from not yet proven, if its bundle is here; else None.

    ⛔ NEVER the highest available. A later bundle that lands before an earlier one does not jump the queue: the
    earlier block would be skipped, and nothing downstream notices a gap (968,984 and 968,986)."""
    want = int(tip_from)
    done = {int(h) for h in proven}
    while want in done:
        want += 1
    return want if want in {int(h) for h in available} else None


BUNDLE_RE = re.compile(r"^leaf_(\d+)\.bin$")


def list_bundles(directory):
    """Heights with a leaf_<h>.bin in `directory` (a missing directory is an empty one)."""
    try:
        names = os.listdir(directory)
    except OSError:
        return set()
    return {int(m.group(1)) for m in map(BUNDLE_RE.match, names) if m}


def block_script(height, gpus, workdir="/workspace", timeout_s=BLOCK_TIMEOUT_S):
    """The bash the pod runs for one block: prove its leaf bundle on every GPU through the CPU product host + hzengine
    (engine.env), check the proof on the pod, and print one TIPDONE line with the measured milliseconds.

    The host's engine backend sizes segments itself (no po2 knob) and picks provers per GPU from the card
    (`hzengine gpu-share`, 3 on 40 GiB and up; ENGINE.md Defaults); only the GPU count is set here."""
    h, w = int(height), workdir
    return "\n".join([
        "#!/bin/bash",
        f"cd {w} || exit 1",
        ". ./engine.env",
        f"export HAZYNC_ENGINE_GPUS={int(gpus)}",
        f"D={w}/tip/{h}; rm -rf \"$D\"; mkdir -p \"$D\" && mv -f {w}/leaf_{h}.bin \"$D\"/ && cd \"$D\" || "
        f"{{ echo \"TIPDONE {h} rc=90 verify=90 ok=0 prove_ms=0 verify_ms=0\"; exit 0; }}",
        "s=$(date +%s%N)",
        f"HAZYNC_BRIDGE_OUT=\"$D\" timeout {int(timeout_s)} {w}/hazync-host prove-range-bridge {h} > prove.log 2>&1; rc=$?",
        "p=$(( ($(date +%s%N) - s) / 1000000 )); s=$(date +%s%N)",
        f"timeout 600 {w}/hazync-host verify-any range_{h}.bin {h} > verify.log 2>&1; v=$?",
        "q=$(( ($(date +%s%N) - s) / 1000000 ))",
        f"grep -q '^RANGE-OK lo={h} hi={h} ' verify.log && ok=1 || ok=0",
        f"echo \"TIPDONE {h} rc=$rc verify=$v ok=$ok prove_ms=$p verify_ms=$q\"",
    ]) + "\n"


TIPDONE_RE = re.compile(r"^TIPDONE (\d+) rc=(\d+) verify=(\d+) ok=([01]) prove_ms=(\d+) verify_ms=(\d+)$")


def parse_tipdone(text, height):
    """The TIPDONE line for `height` in the pod's tip.log, as a dict, or None while the block is still running."""
    for line in (text or "").splitlines():
        m = TIPDONE_RE.match(line.strip())
        if m and int(m.group(1)) == int(height):
            return {"height": int(height), "rc": int(m.group(2)), "verify": int(m.group(3)), "ok": m.group(4) == "1",
                    "prove_s": int(m.group(5)) / 1000.0, "verify_s": int(m.group(6)) / 1000.0}
    return None


def launch_block(workdir, height):
    """Start the block's script detached (sponsor_bot.launch_command's pattern: only setsid -f goes to the background,
    every stream redirected, so the ssh that starts it returns at once)."""
    return (f"cd {workdir} && setsid -f bash {workdir}/tip-{int(height)}.sh >> {workdir}/tip.log 2>&1 < /dev/null; "
            "exit 0")


def kill_script(engine=True):
    """Stop every prover on the pod, carried as BASE64 so the pattern never appears in the remote shell's argv
    (tip_driver.kill_provers: an inline pgrep -f matched the ssh session itself and killed nothing)."""
    names = ("hzengine", "hazync-engine", "hazync-host") if engine else ("hazync-host-cuda",)
    body = "".join(f"for p in $(pgrep -f {n}); do kill -9 $p 2>/dev/null; done\n" for n in names) + "echo KILLED\n"
    blob = base64.b64encode(body.encode()).decode()
    return f"echo {blob} | base64 -d > /tmp/hzkill.sh && sh /tmp/hzkill.sh; rm -f /tmp/hzkill.sh"


def harvest_cmd(workdir="/workspace"):
    """Pack the run's evidence on the pod: every block's logs and proof file, the tip log, the boot log."""
    return (f"cd {workdir} && tar czf tip-harvest.tgz --ignore-failed-read tip.log boot.log sponsor-run.log "
            "tip/*/prove.log tip/*/verify.log tip/*/range_*.bin 2>/dev/null; "
            "sha256sum tip-harvest.tgz | cut -c1-64")


def local_verify(host, range_path, height, vk, pin, run=subprocess.run):
    """Verify a harvested proof HERE, under the release's key file and its pin (engine_backend.rs board key).
    Returns (ok, detail). Without a key and pin there is no pinned check, and that is a refusal, not a pass."""
    if not (vk and pin):
        return False, "no HAZYNC_ENGINE_VK / HAZYNC_ENGINE_VK_PIN: no pinned check"
    env = dict(os.environ, HAZYNC_ENGINE_VK=vk, HAZYNC_ENGINE_VK_PIN=pin)
    try:
        r = run([host, "verify-any", range_path, str(int(height))], capture_output=True, text=True, timeout=600,
                env=env)
    except (OSError, subprocess.TimeoutExpired) as e:
        return False, f"verify-any did not run: {e}"
    ok = re.search(rf"^RANGE-OK lo={int(height)} hi={int(height)} ", r.stdout or "", re.M) is not None
    return ok and r.returncode == 0, ((r.stdout or "") + (r.stderr or "")).strip()[-200:]


class TeardownSignal(Exception):
    """Raised by a signal handler so the run's finally (harvest, release) runs."""


def install_signal_teardown():
    """Every signal an operator or a supervisor might send becomes an exception into the finally."""
    def handler(signum, _frame):
        raise TeardownSignal(f"signal {signum}")
    sigs = [signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGUSR1, signal.SIGUSR2]
    for s in sigs:
        signal.signal(s, handler)
    return sigs


def harvest(runner, pod, outdir, tries=HARVEST_TRIES, sleep=time.sleep, workdir="/workspace"):
    """Fetch every block's logs and proof file from the pod BEFORE it is released. Returns the local tarball or None.
    ⚠ A sha256 check against the pod's own sum: a truncated copy is a failed copy."""
    import hashlib
    os.makedirs(outdir, exist_ok=True)
    dest = os.path.join(outdir, "tip-harvest.tgz")
    for t in range(1, tries + 1):
        r = runner._ssh(pod, harvest_cmd(workdir), 300)
        want = (r.stdout or "").strip().splitlines()[-1:] if r.returncode == 0 else []
        ip, port = pod.ssh
        c = runner._run(["scp", "-q", *runner._opts(), "-P", str(port), f"root@{ip}:{workdir}/tip-harvest.tgz", dest],
                        300)
        if c.returncode == 0 and want and os.path.isfile(dest):
            with open(dest, "rb") as fh:
                got = hashlib.sha256(fh.read()).hexdigest()
            if got == want[0]:
                log(f"harvested (try {t}, sha {got[:16]})")
                return dest
        log(f"HARVEST FAILED (try {t})")
        sleep(10)
    return None


def run_blocks(*, runner, pod, bundles_dir, tip_from, max_blocks, deadline, rundir, gpus, clock, sleep,
               verify_fn=None, workdir="/workspace"):
    """Prove bundles in height order until max_blocks are done or the deadline passes. Returns [result dicts]. Each
    leg goes to the run's ledger (tip_stage.record): seen -> staged (push), proved (pod), verified (local)."""
    proven, results, seen_at = set(), [], {}
    while len(results) < max_blocks and clock() < deadline:
        avail = list_bundles(bundles_dir)
        for h in avail:
            seen_at.setdefault(h, clock())
        h = next_bundle(avail, proven, tip_from)
        if h is None:
            sleep(POLL_S)
            continue
        src = os.path.join(bundles_dir, f"leaf_{h}.bin")
        size = os.path.getsize(src)
        script = os.path.join(rundir, f"tip-{h}.sh")
        with open(script, "w") as fh:
            fh.write(block_script(h, gpus, workdir))
        t0 = clock()
        ok = (runner._scp(pod, [src, script], workdir + "/").returncode == 0
              and runner._ssh(pod, launch_block(workdir, h)).returncode == 0)
        tip_stage.record(rundir, h, "push", clock() - t0, size, ok=ok, note="bundle + script to the pod")
        if not ok:
            raise RuntimeError(f"could not stage block {h} on {pod.name}")
        done = None
        end = min(deadline, clock() + BLOCK_TIMEOUT_S + 120)
        while done is None and clock() < end:
            sleep(POLL_S)
            r = runner._ssh(pod, f"grep '^TIPDONE {h} ' {workdir}/tip.log 2>/dev/null", 30)
            done = parse_tipdone(r.stdout, h) if r.returncode == 0 else None
        if done is None:
            results.append({"height": h, "ok": False, "why": "no TIPDONE before the deadline"})
            break
        done["wait_s"] = round(t0 - seen_at.get(h, t0), 3)
        tip_stage.record(rundir, h, "prove", done["prove_s"], 0, ok=done["ok"] and done["rc"] == 0,
                         note=f"pod verify rc={done['verify']}")
        if verify_fn is not None and done["ok"]:
            got = os.path.join(rundir, f"range_{h}.bin")
            ip, port = pod.ssh
            t1 = clock()
            c = runner._run(["scp", "-q", *runner._opts(), "-P", str(port),
                             f"root@{ip}:{workdir}/tip/{h}/range_{h}.bin", got], 120)
            vok, detail = verify_fn(got, h) if c.returncode == 0 else (False, "could not fetch the proof")
            tip_stage.record(rundir, h, "verify", clock() - t1, os.path.getsize(got) if os.path.isfile(got) else 0,
                             ok=vok, note=detail)
            done["verified_here"] = vok
        results.append(done)
        log(f"block {h}: {'OK' if done['ok'] else 'FAILED'} prove {done['prove_s']:.1f}s "
            f"(queued {done['wait_s']:.0f} s after its bundle landed){'' if verify_fn is None else ', pinned check ' + str(done.get('verified_here'))}")
        proven.add(h)
        if not done["ok"]:
            break                       # ⛔ a failed block stops the run: the next one would leave a gap behind it
    return results


def cost_quote(blocks, price=MEASURED["price_4x_pro6000_se_usd_hr"], setup_min=15.0, per_block_s=None):
    """A quote from measured numbers only: setup (8-15 min measured for pod setup, gpu_block.sh) plus the measured
    per-block prove time, at the measured pod price. Returns dollars."""
    per = per_block_s if per_block_s is not None else max(MEASURED["prove_969122_s"], MEASURED["prove_976379_s"])
    return price * (setup_min * 60 + blocks * per) / 3600.0


def cmd_plan(a):
    print("tip_engine: PLAN -- nothing is rented and nothing is spent")
    print(f"  pod            {a.gpus}x {a.gpu_type[0]} (secure), one pod for the whole run")
    print(f"  measured       4x PRO 6000 SE ${MEASURED['price_4x_pro6000_se_usd_hr']}/hr; 969,122 in "
          f"{MEASURED['prove_969122_s']} s, 976,379 in {MEASURED['prove_976379_s']} s (gpu_blackwell_4x_969122)")
    print("  quote          the measured price x time; the pod bills by the clock, not by the block:")
    for n in (1, 6, 12):
        print(f"    {n:>2} blocks at the tip  ~${cost_quote(n, per_block_s=600):.2f} (15 min setup + {n} x 600 s, "
              "Bitcoin's target interval: the pod waits for each block)")
    for n in (6, 12):
        print(f"    {n:>2} blocks backlog  ~${cost_quote(n):.2f} (15 min setup + {n} x 38.6 s, bundles already there)")
    print("  ⚠ --minutes bounds the run whatever the quote; a slow block or a late bundle costs clock, not blocks.")
    print("  then: boot (SHA_OK KEYS_OK CUDART_OK GPU_OK, engine image id), prove each bundle from --tip-from in order,")
    print("        harvest logs + proofs, release by prefix, verify each proof here under the pinned key")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd")
    for name in ("plan", "run", "selftest"):
        p = sub.add_parser(name)
        p.add_argument("--gpus", type=int, default=GPU_COUNT)
        p.add_argument("--gpu-type", action="append", default=None, help="RunPod GPU type id (repeat to try several)")
        if name == "run":
            p.add_argument("--live", action="store_true", help="actually rent the pod")
            p.add_argument("--budget-usd", type=float, default=0.0)
            p.add_argument("--minutes", type=int, default=60, help="wall-clock cap (the shorter of this and the budget)")
            p.add_argument("--tip-from", type=int, required=True, help="the first height to prove (never skipped)")
            p.add_argument("--max-blocks", type=int, default=6)
            p.add_argument("--bundles", required=True, help="directory where leaf_<h>.bin bundles appear")
            p.add_argument("--rundir", default=None)
            p.add_argument("--host", default=os.environ.get("HAZYNC_HOST", "hazync-host"),
                           help="local product host for the pinned check (verify-any)")
    a = ap.parse_args(argv)
    if a.gpu_type is None:
        a.gpu_type = list(GPU_TYPES)
    cmd = a.cmd or "plan"
    if cmd == "plan":
        return cmd_plan(a)
    if cmd == "selftest":
        import test_tip_engine
        return test_tip_engine.main([])
    if not a.live:
        print("tip_engine: dry run; add --live with --budget-usd to rent the pod")
        return cmd_plan(a)
    if a.budget_usd <= 0:
        raise SystemExit("tip_engine: --live needs --budget-usd")
    if not (sb.ENGINE and sb.LIBZYNC):
        raise SystemExit("tip_engine: needs HAZYNC_SPONSOR_ENGINE=1 and HAZYNC_LIBZYNC=1 (the engine pod's boot)")
    vk, pin = os.environ.get("HAZYNC_ENGINE_VK"), os.environ.get("HAZYNC_ENGINE_VK_PIN")
    if not (vk and pin):
        raise SystemExit("tip_engine: set HAZYNC_ENGINE_VK and HAZYNC_ENGINE_VK_PIN (the release's key file and its "
                         "SHA-256): every proof is checked here under the pinned key")
    rundir = a.rundir or os.path.join(os.getcwd(), time.strftime("tip-engine-%Y%m%d-%H%M%S"))
    os.makedirs(rundir, exist_ok=False)
    tip_stage.use_run_dir(rundir)
    api = sb.RunPod.from_env()
    runner = sb.SshRunner.from_env()
    runner.prepare()                       # signed manifest, the engine image id, boot.sh: all before a cent is spent
    return live_run(a, api, runner, rundir, vk, pin)


def live_run(a, api, runner, rundir, vk, pin, clock=time.time, sleep=time.sleep, workdir="/workspace"):
    """Rent, boot, prove, harvest, release. Everything after the rental is inside the try, and the finally harvests
    before it releases. Injected api / runner / clock / sleep: test_tip_engine drives this with fakes."""
    # review R3-6 #4: the pod checks every proof under the key the runner pinned from the verified release (engine.env);
    # the local check runs under HAZYNC_ENGINE_VK / _PIN. They must be the same key, or the two checks disagree.
    if getattr(runner, "vk_pin", None) != pin:
        raise SystemExit(f"tip_engine: the release's core key ({str(getattr(runner, 'vk_pin', None))[:16]}...) is not "
                         f"HAZYNC_ENGINE_VK_PIN ({pin[:16]}...): nothing was rented")
    install_signal_teardown()
    prefix = run_prefix()
    with open(os.path.join(rundir, "run_prefix.txt"), "w") as fh:
        fh.write(prefix + "\n")
    name, pod_id, pod, results = prefix + "1", None, None, []
    started = clock()
    try:
        info = api.deploy(name, runner.ssh_pubkey, gpu_types=tuple(a.gpu_type), gpu_count=a.gpus, disk_gb=DISK_GB)
        if info is None:
            log(f"no capacity for {a.gpus}x any of {a.gpu_type}: nothing rented, nothing spent")
            return 0
        pod_id = info["id"]
        if tip_session.is_protected(pod_id):
            raise SystemExit(f"tip_engine: deploy returned a protected pod id {pod_id}")
        pod = sb.Pod(info, clock())
        rate = pod.cost_per_hr
        for _ in range(6):                  # a PROVISIONING pod may answer cost 0: ask the pod itself
            if rate > 0:
                break
            sleep(10)
            rate = float((api.pod(pod_id) or {}).get("cost_per_hr") or 0)
        pod.cost_per_hr = rate
        budget_s = fold_rent.seconds_for_budget(a.budget_usd, rate)
        if budget_s <= 0:
            raise SystemExit("tip_engine: the pod reported no price; refusing to run unbounded")
        run_s = min(budget_s, a.minutes * 60)
        deadline = started + run_s
        log(f"pod {pod.name} {a.gpus}x {info.get('gpu_type')} at ${rate}/hr: {run_s // 60} min")
        while pod.ssh is None and clock() < started + 300:
            pod.ssh = api.ssh_of(pod_id)
            if pod.ssh is None:
                sleep(10)
        if pod.ssh is None:
            raise SystemExit("tip_engine: no ssh port within 5 minutes")
        ok, detail = runner.boot(pod)
        tip_stage.record(rundir, 0, "boot", clock() - started, 0, ok=ok, note=detail)
        if not ok:
            raise SystemExit(f"tip_engine: boot failed: {detail}")
        results = run_blocks(runner=runner, pod=pod, bundles_dir=a.bundles, tip_from=a.tip_from,
                             max_blocks=a.max_blocks, deadline=deadline, rundir=rundir, gpus=a.gpus, clock=clock,
                             sleep=sleep, verify_fn=lambda p, h: local_verify(a.host, p, h, vk, pin), workdir=workdir)
    finally:
        if pod is not None and pod.ssh is not None:
            try:
                runner._ssh(pod, kill_script(True), 60)
            except Exception as e:                                  # noqa: BLE001
                log(f"could not stop the provers: {e}")
            try:
                if harvest(runner, pod, rundir, sleep=sleep, workdir=workdir) is None:
                    log("⛔ HARVEST FAILED on every try: the pod's logs are lost with it")
            except Exception as e:                                  # noqa: BLE001
                log(f"⛔ harvest raised: {e}")
        try:
            done, left, clean = sb.release_until_clean(api, [pod_id] if pod_id else [],
                                                       match=lambda n: n.startswith(prefix),
                                                       skip=tip_session.is_protected, sleep=sleep)
            if left or not clean:
                log(f"⛔⛔ NOT CONFIRMED GONE: {left or prefix} -- TERMINATE BY HAND")
            elif done:
                log(f"released {done}")
        except Exception as e:                                      # noqa: BLE001
            log(f"⛔⛔ could not release {prefix}*: {e} -- CHECK RUNPOD BY HAND")
    good = [r for r in results if r.get("ok") and r.get("verified_here")]
    log(f"{len(good)} of {len(results)} blocks proved and verified under the pinned key")
    return 0 if results and len(good) == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
