#!/usr/bin/env python3
"""Proving on a schedule (#725): the window, the "can this block finish" rule, and handing claims back.

Pure: no window, no clock, no Windows. Times are pinned in Europe/London, which changes its clocks on
29.03.2026 (01:00 GMT -> 02:00 BST) and 25.10.2026 (02:00 BST -> 01:00 GMT), so the daylight-saving nights are
real ones, not invented offsets.

    python3 test_schedule.py             # must PASS
    python3 test_schedule.py --control   # naive wall-clock arithmetic and no midnight crossing: MUST FAIL
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import calendar
import datetime as dt
import sys
import xml.etree.ElementTree as ET

import activity  # noqa: E402
import provewindow as schedule  # noqa: E402  (not "schedule": a PyPI package of that name exists)
import supervisor  # noqa: E402
import winpower  # noqa: E402

CONTROL = "--control" in sys.argv
fails = 0

try:
    from zoneinfo import ZoneInfo
    LON = ZoneInfo("Europe/London")
except Exception as e:                       # noqa: BLE001 - Windows Python has no tz database without tzdata
    LON = None
    print(f"  note: no Europe/London time zone here ({e}); the daylight-saving checks use a fixed table")


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


# ── London local time, as the app's two conversions ────────────────────────────────────────────────
def _offset_h(naive_utc):
    """BST (+1) between the 2026 changes, GMT otherwise: the fallback when zoneinfo has no data."""
    return 1 if dt.datetime(2026, 3, 29, 1) <= naive_utc < dt.datetime(2026, 10, 25, 1) else 0


def from_ts(ts):
    if LON is not None:
        return dt.datetime.fromtimestamp(ts, LON).replace(tzinfo=None)
    u = dt.datetime(1970, 1, 1) + dt.timedelta(seconds=ts)
    return u + dt.timedelta(hours=_offset_h(u))


def to_ts(naive):
    if LON is not None:
        return naive.replace(tzinfo=LON).timestamp()
    for off in (1, 0):                       # the later instant for a time that occurs twice, as fold=0 does
        u = naive - dt.timedelta(hours=off)
        if _offset_h(u) == off:
            return calendar.timegm(u.timetuple())
    return calendar.timegm(naive.timetuple())


def at(y, mo, d, h, mi=0):
    return to_ts(dt.datetime(y, mo, d, h, mi))


if CONTROL:
    # the naive reading: seconds left as wall-clock arithmetic (every night 8 h), and no window past midnight
    def _naive_ends_at(self, ts, from_ts=schedule.local_from_ts, to_ts=schedule.ts_from_local):
        opened = self._opened(from_ts(ts))
        if opened is None:
            return None
        return ts + (self._closes(opened) - from_ts(ts)).total_seconds()
    schedule.Window.ends_at = _naive_ends_at
    schedule.Window.crosses_midnight = property(lambda self: False)
    print("CONTROL: wall-clock arithmetic and no midnight crossing -- the checks below MUST fail")

H = 3600.0
night = schedule.Window(schedule.parse_hhmm("23:00"), schedule.parse_hhmm("07:00"))

print("== a window past midnight ==")
# Wednesday 11.11.2026 (GMT all day)
check(night.is_open(at(2026, 11, 11, 23, 30), from_ts), "23:30 is inside 23:00 to 07:00")
check(night.is_open(at(2026, 11, 12, 6, 59), from_ts), "06:59 the next morning is inside it")
check(not night.is_open(at(2026, 11, 12, 7, 0), from_ts), "07:00 is not: the end is not part of the window")
check(not night.is_open(at(2026, 11, 11, 22, 59), from_ts), "22:59 is not")
check(not night.is_open(at(2026, 11, 11, 12, 0), from_ts), "midday is not")
left = night.seconds_left(at(2026, 11, 12, 1, 0), from_ts, to_ts)
check(left == 6 * H, f"at 01:00 there are 6 h left ({left / H if left is not None else None} h)")
left = night.seconds_left(at(2026, 11, 11, 23, 0), from_ts, to_ts)
check(left == 8 * H, f"at 23:00 the whole 8 h are left ({left / H if left is not None else None} h)")

print("== the days belong to the night the window STARTS ==")
fri = schedule.Window(schedule.parse_hhmm("23:00"), schedule.parse_hhmm("07:00"), days=[4])   # Friday only
check(fri.is_open(at(2026, 11, 13, 23, 30), from_ts), "Friday 23:30 is inside a Friday-night window")
check(fri.is_open(at(2026, 11, 14, 3, 0), from_ts), "Saturday 03:00 is too: it is still Friday's night")
check(not fri.is_open(at(2026, 11, 14, 23, 30), from_ts), "Saturday 23:30 is not")
check(not fri.is_open(at(2026, 11, 13, 3, 0), from_ts), "Friday 03:00 is not: that is Thursday's night")
nxt = fri.next_start(at(2026, 11, 14, 8, 0), from_ts, to_ts)
check(nxt == at(2026, 11, 20, 23, 0), f"after Saturday morning the next one opens on Friday 20.11 at 23:00 "
                                      f"({from_ts(nxt) if nxt else None})")

print("== a window inside one day ==")
day = schedule.Window(schedule.parse_hhmm("09:30"), schedule.parse_hhmm("17:00"), days=range(5))
check(day.is_open(at(2026, 11, 11, 9, 30), from_ts) and not day.is_open(at(2026, 11, 11, 17, 0), from_ts),
      "09:30 opens it, 17:00 closes it")
check(not day.is_open(at(2026, 11, 14, 12, 0), from_ts), "and not on Saturday, with Monday to Friday ticked")
check(day.seconds_left(at(2026, 11, 11, 16, 0), from_ts, to_ts) == H, "at 16:00 there is an hour left")
check(day.seconds_left(at(2026, 11, 11, 18, 0), from_ts, to_ts) is None, "outside it nothing is left")

print("== the nights the clocks change ==")
spring = night.seconds_left(at(2026, 3, 28, 23, 0), from_ts, to_ts)
check(spring == 7 * H, f"Saturday 28.03.2026, 23:00 to 07:00 lasts 7 real hours, the clocks going forward "
                       f"({spring / H if spring is not None else None} h)")
autumn = night.seconds_left(at(2026, 10, 24, 23, 0), from_ts, to_ts)
check(autumn == 9 * H, f"Saturday 24.10.2026, it lasts 9 real hours, the clocks going back "
                       f"({autumn / H if autumn is not None else None} h)")
late = night.seconds_left(at(2026, 3, 29, 3, 0), from_ts, to_ts)          # 03:00 BST, after the jump
check(late == 4 * H, f"at 03:00 BST that morning, 4 h are left ({late / H if late is not None else None} h)")

short = schedule.Window(schedule.parse_hhmm("23:00"), schedule.parse_hhmm("01:30"))
second_pass = at(2026, 10, 25, 1, 15) + H      # 01:15 GMT: the second time the wall clock reads 01:15 that night
check(from_ts(second_pass).hour == 1 and short.is_open(second_pass, from_ts), "the repeated 01:15 is inside 23:00 to 01:30")
left = short.seconds_left(second_pass, from_ts, to_ts)
check(left is not None and 0 < left <= 15 * 60,
      f"and the time left then is the real 15 min, not zero or negative ({left / 60 if left is not None else None} min)")

print("== can a block finish before the window ends ==")
check(schedule.estimate([]) is None, "nothing measured: no estimate")
est = schedule.estimate([600, 620, 5000, 610, 590])
check(est == 610 * schedule.MARGIN + schedule.ALLOWANCE_S,
      f"the median of the last few, so one slow block (a PC that slept mid-proof) does not decide it ({est})")
t = at(2026, 11, 12, 6, 30)                                          # 30 min before 07:00
ok, why = schedule.may_take_block(night, t, [2400, 2500, 2450], from_ts, to_ts)
check(not ok and "30m" in why, f"30 min left and a block takes ~40 min: no claim ({why})")
ok, why = schedule.may_take_block(night, t, [600, 620, 610], from_ts, to_ts)
check(ok, f"30 min left and a block takes ~10 min: claim ({why})")
ok, why = schedule.may_take_block(night, t, [], from_ts, to_ts)
check(not ok, f"nothing measured on this card and under an hour left: no claim ({why})")
ok, why = schedule.may_take_block(night, at(2026, 11, 12, 2, 0), [], from_ts, to_ts)
check(ok, f"nothing measured, 5 h left: claim ({why})")
ok, why = schedule.may_take_block(night, at(2026, 11, 12, 12, 0), [60], from_ts, to_ts)
check(not ok and "outside" in why, f"outside the window: no claim ({why})")
check(schedule.remember(list(range(1, 15)), 99) == list(range(6, 15)) + [99], "only the last ten block times are kept")

print("== the settings ==")
for bad, what in ((("23:00", "23:00"), "start and end the same"), (("25:00", "07:00"), "an hour past 23"),
                  (("11pm", "07:00"), "not hh:mm")):
    try:
        schedule.Window.from_config({"start": bad[0], "end": bad[1]})
        check(False, f"{what}: refused")
    except ValueError as e:
        check(True, f"{what}: refused ({e})")
try:
    schedule.Window.from_config({"start": "23:00", "end": "07:00", "days": []})
    check(False, "no days ticked: refused")
except ValueError as e:
    check(True, f"no days ticked: refused ({e})")
check(night.describe() == "23:00 to 07:00 (next morning), every day", night.describe())

print("== the claimed block is tracked, so a stop can hand it back ==")
a = activity.Activity("run")
a.feed("claimed block 1807 (claim 3fa1c0de3fa1c0de3fa1c0de3fa1c0de; yours for 60 min; if you stop, it reopens by itself)",
       now=1000.0)
check(a.holding == "1807", f"after the claim line the worker holds 1807 ({a.holding})")
check(a.holding_claim == "3fa1c0de3fa1c0de3fa1c0de3fa1c0de", f"and knows which claim, to release only that ({a.holding_claim})")
a.feed("proved range [1807..1807] from bridge bundle in 263.0s -> range_1807.hzk", now=1263.0)
check(a.holding == "1807", "still holding once proved: it is not on the board yet")
a.feed("✓ range 1807: the coordinator re-verified your proof and put it on the board as 'x'.", now=1300.0)
check(a.holding is None and a.last_block_s == 300.0,
      f"on the board: nothing to hand back, and the block took 300 s claim to board ({a.holding}, {a.last_block_s})")
a.feed("claimed block 1808 (yours for 60 min)", now=1400.0)
check(a.holding == "1808" and a.holding_claim is None,
      f"a worker too old to name its claim: the block is tracked, the claim is unknown ({a.holding_claim})")
ev = a.feed("released block 1808: it is open to any prover again", now=1401.0)
check(a.holding is None and ev and "back" in ev, f"the worker's own release clears it ({a.holding}, {ev})")
a.feed("claimed block 1809 (yours for 60 min)", now=1500.0)
a.feed("block 1809 was not released (rate limit); if it is still claimed, it reopens by itself", now=1501.0)
check(a.holding == "1809", "a release that failed does not count as one")

print("== handing back a claim from the app ==")
cmd = supervisor.release_command("py", "C:/Hazync/hazync-worker", 1807, "3fa1c0de3fa1c0de3fa1c0de3fa1c0de")
check(cmd == ["py", "C:/Hazync/hazync-worker", "release", "1807", "3fa1c0de3fa1c0de3fa1c0de3fa1c0de"],
      f"the command names the claim ({cmd})")
check(supervisor.release_command("py", "C:/Hazync/hazync-worker", 1807, None) is None,
      "no claim id: no release (the block reopens by itself), never a release that could free a later claim")
check(supervisor.release_verdict(0, "released block 1807: it is open to any prover again\n")[0], "exit 0: released")
ok, msg = supervisor.release_verdict(1, "unknown command 'release'\n\nhazync — Proof Party contributor CLI.\n")
check(not ok and "too old" in msg, f"an older worker without the command is named as such ({msg})")
ok, msg = supervisor.release_verdict(1, "block 1807 was not released (not your claim)")
check(not ok and "not your claim" in msg, f"a refusal says why ({msg})")
ok, msg = supervisor.release_verdict(1, "block 1807 was not released (not this claim: this key holds 1807 under a later "
                                        "claim than the one this release names, and that claim stays)")
check(not ok and "not this claim" in msg, f"a re-claim left alone is not a release ({msg[:60]})")
ok, msg = supervisor.release_verdict(1, "block 1807 was not released (not your claim)")
check(not ok and "not your claim" in msg, f"a refusal says why ({msg})")

print("== keeping awake, and the wake timer ==")


class K32:
    """Answers as Windows does: the PREVIOUS state (0 for a thread that never set one), or 0 when it fails."""

    def __init__(self, fails=False):
        self.calls, self.state, self.fails = [], 0, fails

    def SetThreadExecutionState(self, flags):
        self.calls.append(flags)
        if self.fails:
            return 0
        prev, self.state = self.state, flags
        return prev


k = K32()
check(winpower.keep_awake(True, _kernel32=k) and k.calls[-1] == 0x80000001,
      f"on: ES_CONTINUOUS | ES_SYSTEM_REQUIRED, and not the display ({hex(k.calls[-1])})")
check(winpower.keep_awake(False, _kernel32=k) and k.calls[-1] == 0x80000000, f"off: ES_CONTINUOUS alone ({hex(k.calls[-1])})")
check(not winpower.keep_awake(True, _kernel32=K32(fails=True)), "a call Windows refuses is reported as refused")
xml = winpower.wake_task_xml(fri)
root = ET.fromstring(xml.split("?>", 1)[1])
ns = {"t": "http://schemas.microsoft.com/windows/2004/02/mit/task"}
check(root.find(".//t:WakeToRun", ns).text == "true", "the task wakes the computer")
check(root.find(".//t:StartBoundary", ns).text == "2026-01-01T23:00:00", "at the window's start, local time (no zone)")
days = [c.tag.split("}")[1] for c in root.find(".//t:DaysOfWeek", ns)]
check(days == ["Friday"], f"on the window's days only ({days})")
check(root.find(".//t:Command", ns).text == "cmd.exe", "and runs nothing of ours")
ran = []
ok, _ = winpower.set_wake_task(fri, run=lambda c: (ran.append(c), (0, ""))[1])
check(ok and ran and ran[0][:4] == ["schtasks", "/Create", "/TN", winpower.TASK_NAME] and "/XML" in ran[0],
      f"registered through schtasks /XML ({ran[0][:4] if ran else None})")
ok, _ = winpower.clear_wake_task(run=lambda c: (1, "FEHLER: Das System kann die angegebene Datei nicht finden."))
check(ok, "removing a wake timer that is not there is fine, in any language (judged by schtasks /Query's exit code)")
asked = []
ok, _ = winpower.clear_wake_task(run=lambda c: (asked.append(c[1]), (0, ""))[1])
check(ok and asked == ["/Query", "/Delete"], f"one that is there is deleted ({asked})")
POWERCFG = """Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)
  Subgroup GUID: 238c9fa8-0aad-41ed-83f4-97be242c8f20  (Sleep)
    Power Setting GUID: bd3b718a-0680-4d9d-8ab2-e1d2b4ac806d  (Allow wake timers)
      Possible Setting Index: 000
      Possible Setting Friendly Name: Disable
      Possible Setting Index: 001
      Possible Setting Friendly Name: Enable
      Possible Setting Index: 002
      Possible Setting Friendly Name: Important Wake Timers Only
    Current AC Power Setting Index: 0x00000001
    Current DC Power Setting Index: 0x00000002
"""
allowed = winpower.wake_timers_allowed(POWERCFG)
check(allowed == {"ac": True, "dc": False}, f"Enable on mains, 'important only' on battery reads as not allowed ({allowed})")
check("battery" in winpower.wake_timer_warning(allowed), "and the app says a laptop has to be plugged in")
check("switched off" in winpower.wake_timer_warning({"ac": False, "dc": False}), "wake timers off: said plainly")

if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail with naive time arithmetic, as they must")
        sys.exit(0)
    print("CONTROL BROKEN: the naive version passed everything")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
