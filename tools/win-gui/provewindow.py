"""Proving on a schedule (#725): when the window is open, and whether a block can finish inside it.

⭐ PURE, AND TESTED WITHOUT A WINDOW OR A CLOCK. Every function takes the time it is asked about as a unix
timestamp, and the two conversions it needs (timestamp <-> local wall-clock time) as arguments, so the tests
can pin a real time zone with real daylight-saving changes. The app passes nothing and gets the machine's own.

## What the window means

A window is a start and an end in LOCAL wall-clock time ("23:00" to "07:00") and the days of the week it may
START on. When the end is earlier than the start the window runs past midnight, and it belongs to the day it
started: "Fri 23:00 to 07:00" runs from Friday night into Saturday morning, and ticking only Friday does not
also open Saturday night. Start and end may not be the same: a window that never closes is no window, and
that is the app without a schedule.

## ⛔ Wall-clock in, real seconds out

The window is set in wall-clock time, because that is what a person means by "overnight". The time LEFT in it
is measured in real seconds, because that is what a proof takes. Converting the end of the window back to a
timestamp through the local time zone does that, so on the night the clocks change a 23:00 to 07:00 window is
seven hours long (spring) or nine (autumn), not eight, and nothing here has to know which night that is.

## Can a block finish before the window ends?

The coordinator hands out the next block only when it is claimed, so its size is not known in advance. What
is known is how long the last few blocks took on this card, and the board proves in height order, so the next
block is a neighbour of the last ones. The estimate is the MEDIAN of the last few (one slow outlier, such as a
PC that slept mid-proof, does not decide it), times a margin, plus a fixed allowance for fetching and checking.
With no measurement yet, a block is started only if at least an hour of the window is left.
"""
import datetime as _dt
import re

DAYS = ("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")   # datetime.weekday() order
ALL_DAYS = frozenset(range(7))

RECENT = 5             # how many of this card's recent blocks the estimate looks at
KEEP = 10              # how many are remembered
MARGIN = 1.25          # the next block may be a little bigger than the last ones
ALLOWANCE_S = 120      # fetching the block's data and the coordinator's check, on top of the proof
UNKNOWN_MIN_S = 3600   # no measurement on this card yet: start a block only with this much window left

_HHMM = re.compile(r"^\s*(\d{1,2}):(\d{2})\s*$")


def local_from_ts(ts):
    """The machine's own local wall-clock time for a timestamp, as a naive datetime."""
    return _dt.datetime.fromtimestamp(ts)


def ts_from_local(dt):
    """The timestamp of a naive local wall-clock time, through the machine's own time zone (mktime)."""
    return dt.timestamp()


def parse_hhmm(text):
    """'23:00' -> datetime.time(23, 0). Raises ValueError with a sentence a person can act on."""
    m = _HHMM.match(str(text or ""))
    if not m or int(m.group(1)) > 23 or int(m.group(2)) > 59:
        raise ValueError(f"'{text}' is not a time: use 24-hour hh:mm, for example 23:00")
    return _dt.time(int(m.group(1)), int(m.group(2)))


class Window:
    """A proving window: start and end as datetime.time, and the weekdays (0 = Monday) it may start on."""

    def __init__(self, start, end, days=ALL_DAYS):
        if start == end:
            raise ValueError("the window has to end at a different time from when it starts")
        days = frozenset(int(d) for d in days)
        if not days or not days <= ALL_DAYS:
            raise ValueError("pick at least one day for the window")
        self.start, self.end, self.days = start, end, days

    @classmethod
    def from_config(cls, cfg):
        """From the app's saved settings: {"start": "23:00", "end": "07:00", "days": [0..6]}."""
        return cls(parse_hhmm(cfg.get("start")), parse_hhmm(cfg.get("end")),
                   cfg.get("days", list(range(7))))

    @property
    def crosses_midnight(self):
        return self.end < self.start

    def describe(self):
        """'23:00 to 07:00 (next morning), every day' — for the settings page and the feed."""
        span = f"{self.start:%H:%M} to {self.end:%H:%M}" + (" (next morning)" if self.crosses_midnight else "")
        if self.days == ALL_DAYS:
            when = "every day"
        elif self.days == frozenset(range(5)):
            when = "Monday to Friday"
        elif self.days == frozenset({5, 6}):
            when = "at weekends"
        else:
            when = "starting " + ", ".join(DAYS[d] for d in sorted(self.days))
        return f"{span}, {when}"

    # ── the window that contains a moment ───────────────────────────────────────────────────────────
    def _opened(self, d):
        """The local start (naive datetime) of the window that contains local time `d`, or None."""
        t = d.time()
        if not self.crosses_midnight:
            if self.start <= t < self.end and d.weekday() in self.days:
                return _dt.datetime.combine(d.date(), self.start)
            return None
        if t >= self.start and d.weekday() in self.days:
            return _dt.datetime.combine(d.date(), self.start)
        y = d.date() - _dt.timedelta(days=1)
        if t < self.end and y.weekday() in self.days:
            return _dt.datetime.combine(y, self.start)
        return None

    def _closes(self, opened):
        day = opened.date() + _dt.timedelta(days=1 if self.crosses_midnight else 0)
        return _dt.datetime.combine(day, self.end)

    def is_open(self, ts, from_ts=local_from_ts):
        return self._opened(from_ts(ts)) is not None

    def ends_at(self, ts, from_ts=local_from_ts, to_ts=ts_from_local):
        """Timestamp the window containing `ts` closes at, or None when `ts` is outside every window."""
        opened = self._opened(from_ts(ts))
        if opened is None:
            return None
        end = to_ts(self._closes(opened))
        if end <= ts:
            # The hour the clocks go back happens twice. During its SECOND pass a closing time inside it converts to
            # the first pass, already gone, while the wall clock still reads inside the window. Find the real close
            # by asking the wall clock, a minute at a time (at most the repeated hour).
            t = ts
            for _ in range(180):
                t += 60
                if self._opened(from_ts(t)) is None:
                    return t
        return end

    def seconds_left(self, ts, from_ts=local_from_ts, to_ts=ts_from_local):
        end = self.ends_at(ts, from_ts, to_ts)
        return None if end is None else max(0.0, end - ts)

    def next_start(self, ts, from_ts=local_from_ts, to_ts=ts_from_local):
        """Timestamp the next window opens at, strictly after `ts` (for 'starts in 3h 10m' and the wake timer)."""
        d = from_ts(ts)
        for k in range(0, 8):
            day = d.date() + _dt.timedelta(days=k)
            if day.weekday() not in self.days:
                continue
            at = to_ts(_dt.datetime.combine(day, self.start))
            if at > ts:
                return at
        return None


# ── can a block finish in time ──────────────────────────────────────────────────────────────────────
def remember(durations, seconds, keep=KEEP):
    """This card's recent block times with one more added, newest last, at most `keep`."""
    out = [float(x) for x in (durations or []) if isinstance(x, (int, float)) and x > 0]
    if seconds and seconds > 0:
        out.append(float(seconds))
    return out[-keep:]


def estimate(durations, recent=RECENT):
    """Seconds the next block is expected to take on this card, or None with nothing measured."""
    xs = sorted(float(x) for x in (durations or [])[-recent:] if x and x > 0)
    if not xs:
        return None
    n = len(xs)
    mid = xs[n // 2] if n % 2 else (xs[n // 2 - 1] + xs[n // 2]) / 2
    return mid * MARGIN + ALLOWANCE_S


def may_take_block(window, ts, durations, from_ts=local_from_ts, to_ts=ts_from_local):
    """(yes, reason). Whether to claim another block now: inside the window, with time to finish it."""
    left = window.seconds_left(ts, from_ts, to_ts)
    if left is None:
        return False, "outside the proving window"
    need = estimate(durations)
    if need is None:
        if left >= UNKNOWN_MIN_S:
            return True, "no block measured on this card yet, and over an hour of the window is left"
        return False, (f"only {clock(left)} of the window is left, and no block has been measured on this "
                       f"card yet (I start one only with an hour or more left)")
    if left >= need:
        return True, f"{clock(left)} left; a block takes this card about {clock(need)}"
    return False, (f"only {clock(left)} of the window is left, and a block takes this card about "
                   f"{clock(need)}")


def clock(seconds):
    """'45s', '12m', '3h 05m'. Rounded down to the minute above a minute: these are estimates."""
    s = max(0, int(seconds))
    if s < 60:
        return f"{s}s"
    if s < 3600:
        return f"{s // 60}m"
    return f"{s // 3600}h {(s % 3600) // 60:02d}m"
