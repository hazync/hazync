#!/usr/bin/env python3
"""A transient GitHub 500 must not page a human, and a failed run must not re-rent (sponsor bot).

📏 WHAT HAPPENED, 2026-10-01 15:05:46. The sponsor bot died three seconds into `prepare()`:

    File "/opt/hazync/coordinator/sponsor_bot.py", line 814, in prepare
      urllib.request.urlretrieve(base + f, os.path.join(self.dir, f))
    urllib.error.HTTPError: HTTP Error 500: Internal Server Error

fetching `releases/download/v0.22.1/SHA256SUMS.txt` — GitHub's release CDN, nothing of ours. The
same assets fetched fine minutes later. It cost nothing, because `prepare()` runs before a cent is
spent, and it still woke somebody up.

⛔⛔ AND THE TIMER RE-RAN IT EVERY FIVE MINUTES. Measured on the box: NINE starts in one hour —
14:28, 14:33, 14:39, 14:44, 14:49, 14:55, 15:00, 15:05, 15:11, 15:16. The unit's own comment says
`Restart=no` means it "stops, pages the phone, and waits for a person". It did not wait. `Restart=`
governs systemd's own restarting; a timer is a separate mechanism and ignores it.

⇒ That failure was harmless only because of WHERE it happened. A failure after renting would have
re-rented every five minutes against `--max-usd 50` that each fresh run re-reads as unspent — the
exact scenario `Restart=no` is documented to prevent.

Two guards, and this checks both:
  * `http_retry` — retry 5xx and transport errors, NEVER 4xx, bounded well under the timer interval.
  * a FAILURE LATCH — `ExecStopPost` writes it, `ConditionPathExists=!` makes the next timer fire
    skip quietly instead of starting the bot again.

    python3 test_sponsor_resilience.py             # both guards present and correct
    python3 test_sponsor_resilience.py --control   # neither: one 500 kills the run, nothing latches
"""
import json
import os
import sys
import urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


import sponsor_bot  # noqa: E402

# ⚠ Never actually sleep: the backoff is 2+8+20 s and this must stay a unit test.
slept = []
sponsor_bot.time.sleep = lambda s: slept.append(s)


def http_error(code):
    return urllib.error.HTTPError("https://example/x", code, "boom", None, None)


def flaky(fail_times, exc):
    """A callable that raises `exc` the first `fail_times` calls, then returns 'ok'."""
    state = {"n": 0}

    def fn():
        state["n"] += 1
        if state["n"] <= fail_times:
            raise exc
        return "ok"
    fn.calls = lambda: state["n"]
    return fn


# ⛔ THE CONTROL IS THE SHIPPED BEHAVIOUR: one attempt, no retry, any error kills the run.
def once(fn, what):
    return fn()


retry = once if CONTROL else sponsor_bot.http_retry

# ── 1. a transient 500 must survive ─────────────────────────────────────────────────────────────
slept.clear()
f = flaky(1, http_error(500))
try:
    got = retry(f, "download SHA256SUMS.txt")
    err = None
except BaseException as e:                                   # noqa: BLE001
    got, err = None, e
if CONTROL:
    check(isinstance(err, urllib.error.HTTPError) and err.code == 500,
          f"⛔ control: a single 500 kills the run ({type(err).__name__}) — this is the state that "
          f"paged a phone at 15:05:46 for a CDN hiccup")
else:
    check(got == "ok" and err is None,
          f"one 500 then success is survived ({got!r}, {err!r})")
    check(f.calls() == 2, f"and it took exactly two attempts, not more ({f.calls()})")
    check(slept == [2], f"sleeping the first backoff only ({slept}s)")

# ── 2. ⛔ A 404 IS AN ANSWER, NOT A HICCUP, and must NOT be retried ──────────────────────────────
# This project has published an EMPTY release before (hazync#909: release.sh waited 600 s, published
# nothing and called it verified). Retrying a 404 turns "the release is broken" into "the bot is
# slow", which is far the worse failure.
if not CONTROL:
    slept.clear()
    f404 = flaky(99, http_error(404))
    try:
        retry(f404, "download SHA256SUMS.txt")
        raised = None
    except BaseException as e:                               # noqa: BLE001
        raised = e
    check(isinstance(raised, urllib.error.HTTPError) and raised.code == 404,
          f"a 404 propagates immediately ({type(raised).__name__})")
    check(f404.calls() == 1, f"⛔ and it was tried ONCE — a missing asset is not retried ({f404.calls()})")
    check(slept == [], f"with no backoff at all ({slept})")

    # ── 3. bounded, and bounded under the timer ─────────────────────────────────────────────────
    slept.clear()
    fdead = flaky(99, http_error(503))
    try:
        retry(fdead, "download SHA256SUMS.txt")
        raised = None
    except BaseException as e:                               # noqa: BLE001
        raised = e
    check(isinstance(raised, SystemExit),
          f"a persistent 5xx ends as SystemExit, not a traceback ({type(raised).__name__})")
    check("nothing was spent" in str(raised),
          f"and says nothing was spent, which is true in prepare() ({str(raised)[-40:]!r})")
    check(fdead.calls() == sponsor_bot.HTTP_TRIES,
          f"attempts are bounded at HTTP_TRIES ({fdead.calls()} of {sponsor_bot.HTTP_TRIES})")
    # ⚠ The timer re-fires 5 min after the run goes inactive, so the retry budget must stay well
    # under that or a run could still be waiting when the next one is due.
    check(sum(slept) <= 120,
          f"⚠ total backoff {sum(slept)}s stays far under the timer's 5 min (OnUnitInactiveSec)")

    # ── 4. a transport error retries too — a CDN can refuse the connection rather than answer ───
    slept.clear()
    furl = flaky(2, urllib.error.URLError("connection reset"))
    check(retry(furl, "x") == "ok" and furl.calls() == 3,
          f"a URLError is transient as well ({furl.calls()} attempts)")

# ── 5. the FAILURE LATCH, in the unit ───────────────────────────────────────────────────────────
UNIT = open(os.path.join(HERE, "deploy", "hazync-sponsor-bot.service"), encoding="utf8").read()
cond = "ConditionPathExists=!/var/lib/hazync/sponsor-bot/.failed" in UNIT
post = "ExecStopPost=" in UNIT and "SERVICE_RESULT" in UNIT
if CONTROL:
    check(True, "⛔ control: (the latch is asserted only in the real arm)")
else:
    check(cond, "⛔ the unit refuses to start while the latch file exists")
    check(post, "and writes the latch on any non-success exit, via $SERVICE_RESULT")
    # ⛔ The latch must be written for ANY non-success, not just a non-zero exit: a SIGKILL or a
    # TimeoutStopSec kill must latch too, which is exactly what $SERVICE_RESULT covers and
    # $EXIT_STATUS does not.
    check("$SERVICE_RESULT" in UNIT and "$EXIT_STATUS" not in UNIT,
          "⚠ keyed on $SERVICE_RESULT, so a signal or a timeout latches as well as a bad exit")
    # ⚠ The latch lives under the one path the sandbox can write.
    check("ReadWritePaths=/var/lib/hazync/sponsor-bot" in UNIT,
          "and the latch path is inside ReadWritePaths, or ExecStopPost could not write it")
    # ⚠ A failure to write the latch must not itself page anyone.
    line = [l for l in UNIT.splitlines() if l.startswith("ExecStopPost=")][0]
    check(line.startswith("ExecStopPost=-"),
          f"⚠ and the latch write is prefixed `-`, so failing to latch does not fail the unit ({line[:28]}…)")
    check("rm /var/lib/hazync/sponsor-bot/.failed" in UNIT,
          "⚠ and the unit documents how to clear it, since a latched unit starts silently doing nothing")

# ── 4. ⛔ A FRESH release with no signed manifest yet is BEING SIGNED: skip, don't latch (11-14) ─────────
# release.sh publishes a release and signs it minutes later. At 07:25 UTC on 11-14 a run found v0.25.0 "latest" with
# no SHA256SUMS.txt, died on the 404 and latched the bot off for hours. An OLD release with no manifest is still
# broken and must still fail loudly (section 2's rule).
if not CONTROL:
    import io, json as _json, tempfile, time as _time
    fresh = _time.strftime("%Y-%m-%dT%H:%M:%SZ", _time.gmtime(_time.time() - 300))
    stale = _time.strftime("%Y-%m-%dT%H:%M:%SZ", _time.gmtime(_time.time() - 3 * 86400))
    now = _time.time()
    check(sponsor_bot.release_is_being_signed(fresh, now), "a release published 5 min ago is being signed")
    check(not sponsor_bot.release_is_being_signed(stale, now), "one published 3 days ago is not")
    check(not sponsor_bot.release_is_being_signed("", now) and not sponsor_bot.release_is_being_signed("junk", now),
          "an unknown or unparseable time is NOT fresh (fail loudly, as before)")

    def run_prepare(published_at):
        home = tempfile.mkdtemp()
        key = os.path.join(home, "id_ed25519")
        for f in (key, key + ".pub"):
            open(f, "w").write("ssh-ed25519 AAAA test\n")
        r = sponsor_bot.SshRunner(key, home, "http://meta", None, gnupghome=None)
        def urlopen(req, timeout=None):
            url = req if isinstance(req, str) else req.full_url
            body = ({"method_id": "8" * 64} if url == "http://meta"
                    else {"tag_name": "v9.9.9", "published_at": published_at})
            return io.BytesIO(_json.dumps(body).encode())
        def urlretrieve(url, path):
            raise http_error(404)
        o1, o2 = sponsor_bot.urllib.request.urlopen, sponsor_bot.urllib.request.urlretrieve
        sponsor_bot.urllib.request.urlopen, sponsor_bot.urllib.request.urlretrieve = urlopen, urlretrieve
        try:
            r.prepare()
            return "returned"
        except SystemExit as e:
            return f"exit {e.code}"
        except urllib.error.HTTPError as e:
            return f"http {e.code}"
        finally:
            sponsor_bot.urllib.request.urlopen, sponsor_bot.urllib.request.urlretrieve = o1, o2
    got = run_prepare(fresh)
    check(got == "exit 0", f"⛔ prepare() on a fresh unsigned release exits 0: no latch, nothing started ({got})")
    got = run_prepare(stale)
    check(got == "http 404", f"and on an OLD release with no manifest it still fails on the 404 ({got})")

# ── 5. ⛔ A TRANSIENT pre-rental failure skips without latching, but a STREAK still latches (11-16) ────────
# 05:46 on 11-16: /api/meta timed out 4x in ~30 s while the box was thrashing, and one run latched the bot off
# although "nothing was started and nothing was spent". Real danger (money spent, pods possibly live) still latches.
if not CONTROL:
    import tempfile as _tf
    sb = sponsor_bot

    # 5a. classification: http_retry exhaustion is transient; a CoordError is transient only without an answer / on 5xx
    try:
        sb.http_retry(flaky(99, TimeoutError("timed out")), "GET http://127.0.0.1:8899/api/meta")
        raised = None
    except BaseException as e:                               # noqa: BLE001
        raised = e
    check(isinstance(raised, sb.TransientError), f"4 timeouts end as TransientError ({type(raised).__name__})")

    class _K:                                                # a signing key that signs anything
        def sign(self, m): return b"\0" * 64
    def coord_err(exc):
        c = sb.Coord.__new__(sb.Coord); c.url, c.timeout, c.sk = "http://c", 1, _K()
        o = sb.urllib.request.urlopen
        def boom(*a, **k): raise exc
        sb.urllib.request.urlopen = boom
        try:
            c.queue(); return None
        except sb.CoordError as e:
            return e
        finally:
            sb.urllib.request.urlopen = o
    check(coord_err(TimeoutError("timed out")).transient, "a coordinator timeout is transient")
    check(coord_err(urllib.error.URLError(ConnectionRefusedError(111, "refused"))).transient,
          "a refused connection is transient")
    check(coord_err(http_error(503)).transient, "a coordinator 503 is transient")
    check(not coord_err(http_error(403)).transient, "⛔ a 403 (key refused) is NOT transient: it latches")

    # 5b. one transient run: exit 75, no latch; the unit counts 75 as success
    home = _tf.mkdtemp()
    t0 = 1_000_000.0
    rc = sb.transient_exit("sponsor_bot: GET meta failed", home, now=t0)
    check(rc == sb.TRANSIENT_EXIT == 75, f"one transient timeout → exit 75, skipped, no latch ({rc})")
    _sec, _svc = None, []
    for _ln in UNIT.splitlines():
        _ln = _ln.strip()
        if _ln.startswith("["):
            _sec = _ln
        elif _sec == "[Service]" and _ln.startswith("SuccessExitStatus="):
            _svc += _ln.split("=", 1)[1].split()
    check("75" in _svc, f"⛔ and the unit's [Service] counts 75 as success: no ExecStopPost latch, no OnFailure page ({_svc}). "
          "⚠ a drop-in on the box could reset SuccessExitStatus=; check `systemctl show -p SuccessExitStatus` there")

    # 5c. N in a row over M minutes: latch (SystemExit with a message ⇒ exit 1 ⇒ latch + OnFailure alert)
    outs = []
    for i in range(1, 12):
        try:
            outs.append(sb.transient_exit("sponsor_bot: GET meta failed", home, now=t0 + i * 330))
        except SystemExit as e:
            outs.append(e)
            break
    last = outs[-1]
    check(isinstance(last, SystemExit) and isinstance(last.code, str) and "Latching" in last.code,
          f"a streak latches ({type(last).__name__}: {str(getattr(last, 'code', last))[-60:]!r})")
    runs = len(outs) + 1
    check(runs == sb.TRANSIENT_LATCH_RUNS, f"after exactly {sb.TRANSIENT_LATCH_RUNS} runs in a row ({runs})")
    # 6 fast runs are NOT enough on their own: the streak must also span TRANSIENT_LATCH_S
    home2 = _tf.mkdtemp()
    fast = [sb.transient_exit("x", home2, now=t0 + i) for i in range(10)]
    check(all(r == 75 for r in fast), "10 runs inside a minute do not latch (the span bound holds too)")
    # a run that gets past every pre-rental check clears the streak
    sb.transient_clear(home)
    check(sb.transient_exit("x", home, now=t0 + 99999) == 75, "a clean run resets the streak")
    # fail closed: no state directory ⇒ latch at once
    try:
        sb.transient_exit("x", None); r = "no latch"
    except SystemExit as e:
        r = "latch"
    check(r == "latch", f"⛔ with nowhere to count, it latches as before ({r})")

    # 5d. state-file hygiene: reset on latch, stale streaks restart, clock stepping back, hard cap, junk file
    h = _tf.mkdtemp(); st = os.path.join(h, sb.TRANSIENT_STATE)
    for i in range(5):
        sb.transient_exit("x", h, now=t0 + i * 330)
    try:
        sb.transient_exit("x", h, now=t0 + 5 * 330); r = "no latch"
    except SystemExit:
        r = "latch"
    check(r == "latch" and not os.path.exists(st), f"latching removes transient.json ({r}, exists={os.path.exists(st)})")
    check(sb.transient_exit("x", h, now=t0 + 6 * 330) == 75,
          "⛔ so after a human clears .failed, the next blip starts a NEW streak instead of re-latching at once")
    sb.transient_exit("x", h, now=t0 + 7 * 330)
    check(sb.transient_exit("x", h, now=t0 + 7 * 330 + sb.TRANSIENT_STALE_S + 60) == 75
          and json.load(open(st))["runs"] == 1, "a streak whose last failure is stale starts afresh")
    h = _tf.mkdtemp()
    for i in range(5):
        sb.transient_exit("x", h, now=t0 + i * 330)
    try:
        sb.transient_exit("x", h, now=t0 - 86400); r = "no latch"
    except SystemExit:
        r = "latch"
    check(r == "latch", f"⛔ a clock stepped backwards cannot hold off the latch ({r})")
    h = _tf.mkdtemp(); outs = []
    for i in range(20):
        try:
            outs.append(sb.transient_exit("x", h, now=t0 + i))
        except SystemExit:
            break
    check(len(outs) + 1 == sb.TRANSIENT_LATCH_HARD, f"hard cap: {sb.TRANSIENT_LATCH_HARD} in a row latch whatever the span ({len(outs) + 1})")
    for junk in ("[1,2]", "null", '{"first": NaN, "last": Infinity, "runs": 3}', '{"first": "a", "runs": true}', "{{{"):
        h = _tf.mkdtemp(); open(os.path.join(h, sb.TRANSIENT_STATE), "w").write(junk)
        try:
            ok = sb.transient_exit("x", h, now=t0) == 75 and json.load(open(os.path.join(h, sb.TRANSIENT_STATE)))["runs"] == 1
        except BaseException as e:                         # noqa: BLE001
            ok = False
        check(ok, f"a malformed transient.json ({junk}) is treated as no streak, not a crash")

    # 5e. L4/L3: a LOCAL OSError (disk full) is not transient; an http.client protocol error is
    try:
        sb.http_retry(flaky(99, OSError(28, "No space left on device")), "download SHA256SUMS.txt"); r = None
    except BaseException as e:                             # noqa: BLE001
        r = e
    check(type(r) is OSError, f"⛔ ENOSPC is not retried as transient: it propagates and latches ({type(r).__name__})")
    import http.client as _hc
    check(coord_err(_hc.RemoteDisconnected("gone")).transient, "a coordinator that hangs up mid-answer is transient")

# ── 6. main() end to end: the wiring, the streak file, and ⛔ a post-rental failure latching ───────────────────
if not CONTROL:
    import json as _j
    class FakeCoord:
        fail_queue = False
        def queue(self):
            if FakeCoord.fail_queue:
                raise sb.CoordError("the coordinator could not be reached for GET /api/bot/queue: timed out", transient=True)
            return {"sponsorships": []}
    class FakeRunner:
        def __init__(self, home): self.home = home
        def prepare(self): return self
    class FakeApi:
        def __init__(self): self.terminated = []
        def pods(self): return []
        def terminate(self, pid): self.terminated.append(pid)
    api = FakeApi()
    seed = {"pod": False}
    class SeededBot(sb.Bot):
        """A bot that has ALREADY rented one pod when its loop starts (the post-rental case)."""
        def __init__(self, *a, **k):
            k["sleep"] = lambda s: None
            super().__init__(*a, **k)
            if seed["pod"]:
                self._n = 1
                p = sb.Pod({"id": "pod-1", "name": self.prefix + "1", "cost_per_hr": 0.5}, time.time())
                p.state = "idle"
                self.pods.append(p)
    import time
    loop_reconcile = {"fail": False}
    def fake_reconcile(coord):
        if loop_reconcile["fail"]:
            raise sb.CoordError("the coordinator could not be reached for POST /api/bot/reconcile: timed out", transient=True)
        return []
    saved = {k: getattr(sb, k) for k in ("Bot", "connect_path", "SshRunner", "RunPod", "install_signal_handlers",
                                         "reconcile", "reconcile_work", "pending_work", "terminate_confirmed")}
    saved_coord = sb.Coord.from_env
    def main_run(home, argv=("run", "--live", "--max-pods", "1", "--max-usd", "5", "--max-usd-per-hour", "2", "--tick", "0")):
        os.environ["SPONSOR_BOT_HOME"] = home
        sb.Coord.from_env = classmethod(lambda cls: FakeCoord())
        sb.Bot, sb.connect_path = SeededBot, lambda: os.path.join(home, "bot.db")
        sb.SshRunner = type("SR", (), {"from_env": staticmethod(lambda: FakeRunner(home))})
        sb.RunPod = type("RP", (), {"from_env": staticmethod(lambda: api)})
        sb.install_signal_handlers = lambda: None
        sb.reconcile, sb.reconcile_work, sb.pending_work = fake_reconcile, (lambda c, coord: 0), (lambda *a, **k: [])
        try:
            return sb.main(list(argv))
        except SystemExit as e:
            return e.code if isinstance(e.code, int) else 1     # python exits 1 for a message
        finally:
            sb.Coord.from_env = saved_coord
            for k, v in saved.items():
                setattr(sb, k, v)
    home = _tf.mkdtemp(); st = os.path.join(home, sb.TRANSIENT_STATE)
    FakeCoord.fail_queue = True
    rc = main_run(home)
    check(rc == 75 and os.path.exists(st) and _j.load(open(st))["runs"] == 1,
          f"main(): a pre-rental coordinator timeout exits 75 and starts the streak (rc={rc})")
    _j.dump({"first": time.time() - 30 * 60, "last": time.time() - 60, "runs": 5}, open(st, "w"))
    rc = main_run(home)
    check(rc == 1 and not os.path.exists(st), f"main(): the 6th in a row exits 1 (⇒ latch + alert) ({rc})")
    sb.transient_exit("x", home)
    FakeCoord.fail_queue = False
    rc = main_run(home)
    check(rc == 0 and not os.path.exists(st), f"main(): a clean run exits 0 and removes the streak file ({rc})")
    sb.transient_exit("x", home)
    FakeCoord.fail_queue = True
    rc = main_run(home, ("trial", "--blocks", "5", "--live", "--max-pods", "1", "--max-usd", "5", "--max-usd-per-hour", "2"))
    check(rc == 75 and _j.load(open(st))["runs"] == 1, f"a manual trial keeps its own streak, not the timer's ({rc})")

    # ⛔ post-rental: a pod is live, the coordinator times out in the LOOP ⇒ pods terminated, NOT exit 75
    home = _tf.mkdtemp(); FakeCoord.fail_queue = False; seed["pod"] = True; loop_reconcile["fail"] = True
    api.terminated.clear()
    rc = main_run(home)
    check(rc == 6, f"⛔ a transient coordinator failure AFTER a pod was rented is the loop's own 'coordinator' stop "
                   f"(exit 6 ⇒ latch), never a transient skip ({rc})")
    check(api.terminated == ["pod-1"], f"⛔ and the rented pod was terminated ({api.terminated})")
    check(not os.path.exists(os.path.join(home, sb.TRANSIENT_STATE)), "and it did not count as a transient streak")
    # ⛔ M1, structural: even if a TransientError ever escaped the loop after a deploy (a future regression),
    # main() must refuse exit 75 because this run rented something.
    api.terminated.clear(); loop_reconcile["fail"] = False
    def leak(self, c, now):
        raise sb.TransientError("sponsor_bot: a transient error that leaked out of the loop")
    o_obs = SeededBot._observe
    SeededBot._observe = leak
    try:
        rc = main_run(_tf.mkdtemp())
    finally:
        SeededBot._observe = o_obs
    check(rc == 1 and api.terminated == ["pod-1"],
          f"⛔ a TransientError escaping AFTER a rental latches (exit 1) and the pod is terminated ({rc}, {api.terminated})")
    seed["pod"] = False; loop_reconcile["fail"] = False
    os.environ.pop("SPONSOR_BOT_HOME", None)

print()
if CONTROL:
    if fails:
        print(f"FAIL (control): {len(fails)} — the shipped shape must die on one 500")
        sys.exit(1)
    print("PASS (control): no retry, so a CDN hiccup ends the run and pages a phone")
    sys.exit(0)
if fails:
    print(f"FAIL {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("PASS (real)")
