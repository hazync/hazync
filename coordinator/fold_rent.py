#!/usr/bin/env python3
"""Rent ONE GPU, fold for a bounded time, measure what it cost. (hazync#631 follow-on)

⛔⛔ WHY THIS EXISTS, AND WHY IT IS NOT sponsor_bot. The spine is 7,831 steps behind the frontier and
EVERY available step is width 1 -- measured 2026-10-06, `widest single step: 1` across all 7,831 --
because nothing has folded that region. Absorbing is serial and only the leftmost range can be
anchored; folding is embarrassingly parallel and anyone can do it. A balanced fold tree over N blocks
costs N-1 folds plus ONE absorb, against N absorbs, so folding does not reduce the work -- it MOVES
it into the job that can be bought in parallel.

sponsor_bot rents pods beautifully and then runs `hazync-worker run <height>`, one assigned block at
a time, because that is what a sponsorship is. This reuses its RunPod client, its SshRunner (signed
release, SHA_OK/GPU_OK/METHOD_ID boot checks) and its terminate path, and runs `hazync-worker fold`
in a bounded loop instead.

⛔ MONEY. `--live` is off by default and does nothing without `--budget-usd`. The budget is converted
to WALL CLOCK from the pod's own cost_per_hr the moment it is known, with a safety margin, because a
budget that is only checked between folds is not a budget -- one long fold would overrun it.

⛔⛔ THE POD IS TERMINATED ON EVERY EXIT PATH, and the log is HARVESTED FIRST. hazync has burned ~$45
across six launches for zero blocks, and a separate incident tore pods down with no log fetch and
spent ten days blaming "nobody ran it". Evidence before teardown, always.

    fold_rent.py plan                                   # what it WOULD do; spends nothing
    fold_rent.py run --live --budget-usd 10 --identity ghost-rented
    fold_rent.py selftest                               # the arithmetic and the teardown paths
"""
import argparse
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

# ⚠ Imported, not copied. sponsor_bot's boot verifies the release signature, the GPU and the
# METHOD_ID; reimplementing any of that here would mean two things to keep correct.
import sponsor_bot as sb  # noqa: E402

# ⚠ A fold is small next to a block prove, so the poll is tight enough to notice progress but not so
# tight that it hammers ssh on a box we are paying for by the second.
POLL_S = 20
# ⚠ Margin on the wall-clock budget. Teardown itself takes time, and a pod billed past the cap
# because the terminate call was still in flight is money spent on nothing.
BUDGET_MARGIN = 0.85


def seconds_for_budget(budget_usd, cost_per_hr):
    """How long we may run, from the pod's OWN price. (0 when the price is unknown.)

    ⛔ NEVER GUESS THE PRICE. An unknown cost_per_hr means an unbounded spend, so it yields zero
    seconds and the caller must refuse to start, rather than running on an assumed rate.
    """
    if not cost_per_hr or cost_per_hr <= 0:
        return 0
    return int((budget_usd / cost_per_hr) * 3600 * BUDGET_MARGIN)


def fold_script(identity_tag, max_seconds):
    """The bash the pod runs: fold until the clock runs out, one line per attempt.

    ⚠ `hazync-worker fold` asks the coordinator for work and exits; it does not loop. So the loop is
    here, and it checks the deadline BEFORE each fold rather than after, so the last fold cannot
    start one second before the cap and run past it.
    """
    return "\n".join([
        "#!/bin/bash",
        "cd /workspace",
        f"end=$(( $(date +%s) + {int(max_seconds)} ))",
        "n=0",
        'while [ "$(date +%s)" -lt "$end" ]; do',
        "  n=$((n+1))",
        f"  HAZYNC_HOME=/root/.hazync-ids/{identity_tag} BUNDLE_DIR=/workspace/bundles "
        "WITNESS_DIR=/workspace/witnesses HAZYNC_HOST=/workspace/hazync-host-cuda "
        "./hazync-worker fold; rc=$?",
        '  echo "FOLD $n rc=$rc $(date -u +%FT%TZ)"',
        # ⚠ A coordinator with no claimable fold work answers immediately and forever. Backing off
        # on failure stops a dry spell becoming thousands of log lines and a hot loop on ssh.
        '  [ "$rc" -ne 0 ] && sleep 10',
        "done",
        "echo ALLDONE",
    ]) + "\n"


def start_fold(runner, pod, identity_tag, max_seconds):
    """Copy ONE identity, push the fold loop, launch it detached. Modelled on SshRunner.start.

    ⛔ IT IS NOT SshRunner.start. That one assigns blocks from a sponsorship and runs them once each;
    this folds whatever the coordinator offers, repeatedly, until the clock runs out. Keeping it here
    rather than patching sponsor_bot means the live money-spender is untouched by this experiment.
    """
    home = os.path.join(os.environ["SPONSOR_BOT_HOME"], "identities", identity_tag)
    if not os.path.isfile(os.path.join(home, "key.hex")):
        raise SystemExit(f"fold_rent: no identity at {home} (needs key.hex and handle)")
    dest = f"/root/.hazync-ids/{identity_tag}"
    if (runner._ssh(pod, f"mkdir -p {dest} && chmod 700 {dest}").returncode != 0
            or runner._scp(pod, [os.path.join(home, "key.hex"), os.path.join(home, "handle")],
                           dest + "/").returncode != 0
            or runner._ssh(pod, f"chmod 600 {dest}/key.hex").returncode != 0):
        raise SystemExit(f"fold_rent: could not copy identity {identity_tag} to {pod.name}")
    script = os.path.join(runner.dir, f"fold-{pod.id}.sh")
    with open(script, "w") as f:
        f.write(fold_script(identity_tag, max_seconds))
    if runner._scp(pod, [script], "/workspace/sponsor-run.sh").returncode != 0:
        raise SystemExit(f"fold_rent: could not copy the fold loop to {pod.name}")
    r = runner._ssh(pod, sb.launch_command())
    if r.returncode != 0:
        raise SystemExit(f"fold_rent: could not start folding on {pod.name}: {r.stderr.strip()[:200]}")


def summarise(log_text):
    """folds attempted, folds that succeeded — from the log the pod actually wrote."""
    attempts = [l for l in log_text.splitlines() if l.startswith("FOLD ")]
    ok = [l for l in attempts if " rc=0 " in l]
    return len(attempts), len(ok)


def cmd_plan(a):
    print("fold_rent: PLAN — nothing is rented and nothing is spent")
    print(f"  identity tag   {a.identity}")
    print(f"  budget         ${a.budget_usd:.2f}" if a.budget_usd else "  budget         <unset>")
    for rate in (0.20, 0.50, 1.00, 2.00):
        s = seconds_for_budget(a.budget_usd or 10.0, rate)
        print(f"    at ${rate:>4.2f}/hr -> {s // 60} min of folding")
    print("  ⚠ the real rate comes from the pod itself; these are only the shape of the trade")
    print("  then: boot (SHA_OK, GPU_OK, METHOD_ID), fold until the clock runs out,")
    print("        harvest /workspace/sponsor-run.log, terminate, print $/fold")
    return 0


def cmd_selftest(a):
    """⛔ THE PARTS THAT SPEND MONEY MUST BE EXERCISED WITHOUT SPENDING ANY."""
    fails = []

    def check(ok, what):
        print(("  ok   " if ok else "  FAIL ") + what)
        if not ok:
            fails.append(what)

    # 1. the budget is a wall clock, derived from the real price
    check(seconds_for_budget(10, 1.0) == int(10 * 3600 * BUDGET_MARGIN),
          "ten dollars at $1.00/hr is ten hours, less the teardown margin")
    check(seconds_for_budget(10, 2.0) < seconds_for_budget(10, 1.0),
          "a dearer card buys less time")
    # 2. ⛔ AN UNKNOWN PRICE IS NOT A FREE POD
    check(seconds_for_budget(10, 0) == 0, "⛔ an unknown price yields ZERO seconds, never a default")
    check(seconds_for_budget(10, None) == 0, "⛔ and a missing price does too")
    # 3. the margin genuinely reserves time for teardown
    check(seconds_for_budget(10, 1.0) < 10 * 3600,
          "the cap is strictly under the budget, leaving room to terminate")
    # ⛔ THE CLOCK CAP MUST WIN WHEN IT IS SHORTER. $10 at $0.40/hr is 21 hours; a 45-minute test
    # must not become an overnight rental because the budget happened to be generous.
    cheap = seconds_for_budget(10, 0.40)
    check(cheap > 45 * 60, f"a cheap card would otherwise run {cheap // 3600}h on $10")
    check(min(cheap, 45 * 60) == 45 * 60, "⛔ the wall-clock cap wins when it is the shorter of the two")
    # 4. the script checks the deadline BEFORE folding, not after
    s = fold_script("tag", 600)
    check("while [ \"$(date +%s)\" -lt \"$end\" ]" in s,
          "the loop tests the deadline before each fold, so none can start past the cap")
    check("./hazync-worker fold" in s, "it runs the FOLD job, not run/spine")
    check("/root/.hazync-ids/tag" in s, "it folds as the identity it was given")
    check("echo ALLDONE" in s, "it marks completion, which is how status() knows it finished")
    # 5. the summary counts what happened, not what was hoped
    n, ok = summarise("FOLD 1 rc=0 x\nFOLD 2 rc=1 y\nFOLD 3 rc=0 z\nALLDONE")
    check((n, ok) == (3, 2), f"three attempts, two succeeded (got {n}, {ok})")
    n, ok = summarise("ALLDONE")
    check((n, ok) == (0, 0), "⚠ a run that folded nothing reports zero, not success")

    print()
    if fails:
        print(f"FAIL {len(fails)}")
        return 1
    print("PASS (real)")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd")
    for name in ("plan", "run", "selftest"):
        p = sub.add_parser(name)
        if name in ("plan", "run"):
            p.add_argument("--identity", default="ghost-rented",
                           help="identity TAG under $SPONSOR_BOT_HOME/identities to fold as")
            p.add_argument("--budget-usd", type=float, default=0.0,
                           help="hard cap; converted to wall clock from the pod's own price")
            # ⛔⛔ A BUDGET IS NOT A CLOCK. $10 on a $0.40/hr card is TWENTY-ONE HOURS, so a budget
            # alone would leave a pod folding overnight on what was meant to be a short measurement.
            # The run takes the SHORTER of the two, always.
            p.add_argument("--minutes", type=int, default=45,
                           help="wall-clock cap; the run uses whichever of this and --budget-usd is shorter")
        if name == "run":
            p.add_argument("--live", action="store_true", help="actually rent a GPU")
    a = ap.parse_args(argv)
    cmd = a.cmd or "plan"
    if cmd == "plan":
        return cmd_plan(a)
    if cmd == "selftest":
        return cmd_selftest(a)

    # ---- run ----
    if not a.live:
        print("fold_rent: dry run; add --live with --budget-usd to rent a GPU")
        return cmd_plan(a)
    if a.budget_usd <= 0:
        raise SystemExit("fold_rent: --live needs --budget-usd")

    # ⚠ from_env, not a hand-rolled key read: it refuses on a missing or EMPTY key file by name,
    # which a `open(...).read()` of my own would have turned into an auth failure much later.
    api = sb.RunPod.from_env()
    runner = sb.SshRunner.from_env()
    runner.prepare()

    # ⛔⛔ THE ID IS CAPTURED THE INSTANT THE POD EXISTS, AND TEARDOWN USES ONLY THIS. My first
    # attempt terminated via the pod OBJECT, so when a later line raised (deploy returns a DICT, not
    # a Pod) the finally clause raised too and left a rented GPU billing at $0.74/hr. A teardown that
    # depends on anything built after the rental is not a teardown.
    pod_id = None
    pod = None
    started = None
    rate = 0.0
    log = ""
    try:
        info = api.deploy(f"hazync-fold-{int(time.time())}", runner.ssh_pubkey)
        if info is None:
            print("fold_rent: no GPU capacity in any configured type — nothing rented, nothing spent")
            return 0
        pod_id = info["id"]
        pod = sb.Pod(info, time.time())
        rate = pod.cost_per_hr
        budget_s = seconds_for_budget(a.budget_usd, rate)
        capped_s = min(budget_s, a.minutes * 60) if budget_s else 0
        if budget_s and capped_s < budget_s:
            print(f"  ⚠ ${a.budget_usd:.2f} would buy {budget_s // 60} min; capped to {a.minutes} min")
        budget_s = capped_s
        print(f"pod {pod.name} ({pod.gpu_type}) at ${rate}/hr -> folding for {budget_s // 60} min")
        if budget_s <= 0:
            raise SystemExit("fold_rent: the pod did not report a price; refusing to run unbounded")

        # ⚠ SSH DOES NOT EXIST YET. deploy() returns before the container has a public port, and
        # pod.ssh starts as None — _ssh unpacks it, so calling boot() too early is a TypeError on a
        # pod that is already costing money.
        deadline = time.time() + 300
        while pod.ssh is None and time.time() < deadline:
            for row in api.pods():
                if row.get("id") == pod_id and row.get("ssh"):
                    pod.ssh = row["ssh"]
                    break
            if pod.ssh is None:
                time.sleep(10)
        if pod.ssh is None:
            raise SystemExit("fold_rent: the pod never offered an ssh port within 5 minutes")
        print(f"  ssh up after {int(time.time() - pod.created)}s")

        ok, detail = runner.boot(pod)
        if not ok:
            raise SystemExit(f"fold_rent: boot failed: {detail}")
        start_fold(runner, pod, a.identity, budget_s)
        started = time.time()
        while time.time() - started < budget_s + POLL_S:
            if runner.status(pod) == "finished":
                break
            time.sleep(POLL_S)
    finally:
        if pod is not None and pod.ssh is not None:
            # ⛔ HARVEST BEFORE TEARDOWN. A released pod takes its log with it.
            try:
                log = runner._ssh(pod, "cat /workspace/sponsor-run.log", 60).stdout or ""
            except Exception as e:                                  # noqa: BLE001
                log = f"(could not fetch the log: {e})"
            out = os.path.join(os.environ.get("SPONSOR_BOT_HOME", "."), f"fold-{pod_id}.log")
            try:
                with open(out, "w") as f:
                    f.write(log)
                print(f"log saved: {out}")
            except OSError as e:
                print(f"could not save the log: {e}")
        if pod_id:
            print("terminating…", pod_id)
            try:
                print("  ", sb.terminate_confirmed(api, pod_id))
            except Exception as e:                                  # noqa: BLE001
                # ⛔ SAY IT LOUDLY. A pod that outlived its run bills until someone notices.
                print(f"  ⛔⛔ TERMINATE FAILED for {pod_id}: {e} — TERMINATE IT BY HAND")

    attempts, ok_n = summarise(log)
    elapsed = (time.time() - started) if started else 0.0
    spent = elapsed * (rate or 0) / 3600.0
    print(f"\nfolds attempted {attempts}, succeeded {ok_n}, {elapsed/60:.1f} min, ${spent:.2f}")
    if ok_n:
        print(f"  ${spent/ok_n:.4f} per fold  ⇒  7,831 would cost ${spent/ok_n*7831:.2f}")
    else:
        print("  ⚠ nothing folded — do NOT extrapolate from a run that produced nothing")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
