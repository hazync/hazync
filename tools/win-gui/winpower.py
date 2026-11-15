"""Keeping the PC awake while it proves, and waking it for the proving window (#725). Windows only; a no-op elsewhere.

## Keep awake

`SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` tells Windows not to SLEEP the machine while it
is set. The screen may still turn off, and the lock screen still comes on: that is ES_DISPLAY_REQUIRED, which
is deliberately not asked for. ⚠ It belongs to the THREAD that set it, so it is always set and cleared from the
app's main (window) thread, which lives as long as the app. Closing the app clears it too.

## Wake timer

Waking a sleeping PC needs a scheduled task with "Wake the computer to run this task" ticked. `schtasks` has no
switch for that, so the task is registered from XML. Its only job is to wake the machine: it runs nothing of
ours (`cmd /c exit 0`), and the app, left open, sees the window has started and begins. Two things can stop it,
and the app says so rather than letting someone wait for a morning's work that never happened:
- "Allow wake timers" is off in the power plan (common on laptops on battery, and some PCs ship that way);
- the PC was shut down or hibernated rather than put to sleep, or the app was closed.
"""
import os
import re
import subprocess
import tempfile
from xml.sax.saxutils import escape

IS_WINDOWS = os.name == "nt"

ES_CONTINUOUS = 0x80000000
ES_SYSTEM_REQUIRED = 0x00000001

TASK_NAME = r"Hazync\Wake for proving"
# powercfg's GUIDs for the sleep sub-group and its "Allow wake timers" setting
_SUB_SLEEP = "238c9fa8-0aad-41ed-83f4-97be242c8f20"
_RTC_WAKE = "bd3b718a-0680-4d9d-8ab2-e1d2b4ac806d"


def keep_awake(on, _kernel32=None):
    """Ask Windows not to sleep (on=True) or hand the decision back (on=False). Returns True if it was applied."""
    if not IS_WINDOWS and _kernel32 is None:
        return False
    try:
        k32 = _kernel32
        if k32 is None:
            import ctypes
            k32 = ctypes.windll.kernel32
        flags = ES_CONTINUOUS | (ES_SYSTEM_REQUIRED if on else 0)
        # ⚠ The return value is the PREVIOUS state, which can legitimately be 0, so it is not a success test.
        k32.SetThreadExecutionState(flags)
        return True
    except Exception:          # noqa: BLE001 - power management must never stop the app from proving
        return False


def wake_task_xml(window):
    """The Task Scheduler XML for a task that wakes the PC when `window` (schedule.Window) opens.

    One weekly calendar trigger at the window's start on each day it may start, in LOCAL time (no time-zone
    suffix on StartBoundary, so the clocks changing moves the wake with them)."""
    names = ("Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday")
    days = "".join(f"<{names[d]} />" for d in sorted(window.days))
    start = f"2026-01-01T{window.start:%H:%M}:00"
    return f"""<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{escape("Wakes this PC when the Hazync proving window opens. The Hazync app does the proving; this task runs nothing. Turn it off in Hazync, under Settings.")}</Description>
  </RegistrationInfo>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>{start}</StartBoundary>
      <Enabled>true</Enabled>
      <ScheduleByWeek>
        <DaysOfWeek>{days}</DaysOfWeek>
        <WeeksInterval>1</WeeksInterval>
      </ScheduleByWeek>
    </CalendarTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>true</WakeToRun>
    <ExecutionTimeLimit>PT1M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>cmd.exe</Command>
      <Arguments>/c exit 0</Arguments>
    </Exec>
  </Actions>
</Task>
"""


def _run(cmd, timeout=30):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, errors="replace")
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except (OSError, subprocess.SubprocessError) as e:
        return 1, str(e)


def set_wake_task(window, run=_run):
    """Register (or replace) the wake task for `window`. (ok, message)."""
    if not IS_WINDOWS and run is _run:
        return False, "wake timers are a Windows feature"
    fd, path = tempfile.mkstemp(suffix=".xml", prefix="hazync-wake-")
    try:
        # ⚠ schtasks /XML wants the file in UTF-16, matching the encoding its header names
        with os.fdopen(fd, "w", encoding="utf-16") as fh:
            fh.write(wake_task_xml(window))
        rc, out = run(["schtasks", "/Create", "/TN", TASK_NAME, "/XML", path, "/F"])
    finally:
        try:
            os.remove(path)
        except OSError:
            pass
    if rc != 0:
        return False, f"Windows would not create the wake timer: {out.strip()[-300:]}"
    return True, f"the PC will be woken at {window.start:%H:%M} on the window's days"


def clear_wake_task(run=_run):
    """Remove the wake task if it exists. (ok, message). Removing one that is not there is fine."""
    if not IS_WINDOWS and run is _run:
        return True, "nothing to remove"
    rc, out = run(["schtasks", "/Delete", "/TN", TASK_NAME, "/F"])
    if rc == 0 or "cannot find" in out.lower() or "does not exist" in out.lower():
        return True, "the wake timer is off"
    return False, f"Windows would not remove the wake timer: {out.strip()[-300:]}"


def wake_timers_allowed(powercfg_output):
    """Read `powercfg /query SCHEME_CURRENT SUB_SLEEP RTCWAKE`: {"ac": bool|None, "dc": bool|None}.

    Index 0 is Disable, 1 Enable, 2 "Important wake timers only" — which does NOT include a task like this one,
    so only 1 counts as allowed. None when the setting cannot be read (an older Windows, a locked-down PC)."""
    out = {"ac": None, "dc": None}
    for line in (powercfg_output or "").splitlines():
        m = re.search(r"Current\s+(AC|DC)\s+Power\s+Setting\s+Index:\s*0x([0-9a-fA-F]+)", line)
        if m:
            out[m.group(1).lower()] = int(m.group(2), 16) == 1
    return out


def wake_timer_warning(allowed):
    """A sentence about what will stop the wake timer working, or "" when nothing is known to."""
    ac, dc = allowed.get("ac"), allowed.get("dc")
    if ac is False:
        return ("Wake timers are switched off in this PC's power plan, so it will NOT wake for the window. Turn on "
                "Control Panel > Power Options > Change plan settings > Change advanced power settings > Sleep > "
                "Allow wake timers.")
    if ac is None:
        return "I could not check whether this PC allows wake timers; if it does not wake, look at Allow wake timers in Power Options."
    if dc is False:
        return "On battery this PC does not allow wake timers, so a laptop has to be plugged in to wake for the window."
    return ""


def query_wake_timers(run=_run):
    """The power plan's "Allow wake timers" setting, as wake_timers_allowed() reads it."""
    if not IS_WINDOWS and run is _run:
        return {"ac": None, "dc": None}
    rc, out = run(["powercfg", "/query", "SCHEME_CURRENT", _SUB_SLEEP, _RTC_WAKE])
    return wake_timers_allowed(out if rc == 0 else "")
