"""What the machine is doing while it proves: graphics-card memory, utilisation and temperature, system RAM and CPU.

⭐ ASKED FOR BY THE OWNER, PROVING ON A 4 GB GTX 1050 Ti: "would be good to see gpu memory and cpu memory or stats
during proving as a visual". On a small card the question that matters is how close a block came to the card's
memory, so each block's PEAK is kept and written into the technical log where the block finishes or fails — an
out-of-memory case becomes a number (segment size, peak, card total) instead of a guess.

⭐ PURE, AND TESTED WITHOUT A CARD OR A DISPLAY (test_stats.py). Nothing here imports tkinter. The readers that touch
the OS (`read_cpu_times`, `read_memory`) and the one that runs nvidia-smi (`Sampler`, through an injected `run`) are
thin; the parsing and the maths are plain functions.

⚠ NO NEW DEPENDENCY. psutil is not bundled (hazync-gui.spec), so CPU and RAM come from ctypes on Windows
(GetSystemTimes, GlobalMemoryStatusEx) and /proc on Linux; the GPU numbers from nvidia-smi, which the app already
runs (supervisor.nvidia_smi finds it off PATH too, and winquiet keeps it from flashing a console window).

⚠ WHAT THE NUMBERS ARE. `memory.used` is the WHOLE card's memory in use, the desktop's included, not the prover's
alone; and the peak is the highest of samples taken every POLL_S seconds, so a spike shorter than that can be missed.
The log line says "sampled every 2 s" so nobody reads it as an exact high-water mark.
"""
import ctypes
import sys
import threading
from collections import deque

POLL_S = 2.0
HISTORY_S = 300                              # the chart's span: the last five minutes
HISTORY_N = int(HISTORY_S / POLL_S)

GPU_QUERY = "index,uuid,memory.used,memory.total,utilization.gpu,temperature.gpu"
SMI_TIMEOUT_S = 8                            # nvidia-smi answers in about a second; a stuck one must not pile up

# How the GPU half of the panel stands. `ok`: numbers. `none`: no NVIDIA card or no nvidia-smi (the CPU build's
# machine). `unavailable`: nvidia-smi is there but failed or was slow — said once, never in the log on every poll.
GPU_OK, GPU_NONE, GPU_UNAVAILABLE = "ok", "none", "unavailable"
NO_GPU_TEXT = "CPU build — no GPU"
UNAVAILABLE_TEXT = "GPU stats unavailable"

# After a `none` or `unavailable` answer, ask again only every this many polls: a machine without a card should not
# run a missing program every two seconds, and a failing nvidia-smi should not be hammered.
RETRY_NONE_POLLS = 30
RETRY_UNAVAILABLE_POLLS = 15

# nvidia-smi's own words when it runs but there is nothing to report on. Anything else that fails is `unavailable`.
_NO_DEVICE_MARKERS = ("no devices were found",)


def _num(field):
    """One nvidia-smi field as a number, or None for "[N/A]", "[Not Supported]", "" or anything unreadable."""
    s = (field or "").strip()
    try:
        return float(s) if "." in s else int(s)
    except ValueError:
        return None


def parse_gpus(text):
    """nvidia-smi's CSV for GPU_QUERY (noheader, nounits) -> [{index, uuid, used_mb, total_mb, util, temp}].

    ⚠ A field nvidia-smi cannot read ("[N/A]", common for utilisation and temperature on laptops and some
    older cards) is None, never 0: a card reported at 0 °C or 0 % busy would be a made-up number. A line without a
    numeric index is skipped, since it cannot be matched to the card a worker was pinned to.
    """
    out = []
    for line in (text or "").strip().splitlines():
        parts = [x.strip() for x in line.split(",")]
        if len(parts) < 6 or not parts[0].isdigit():
            continue
        out.append({"index": int(parts[0]), "uuid": parts[1], "used_mb": _num(parts[2]),
                    "total_mb": _num(parts[3]), "util": _num(parts[4]), "temp": _num(parts[5])})
    return out


def classify_smi(rc, out):
    """(state, gpus) from one nvidia-smi run. rc 127 is supervisor._run's "not found"."""
    low = (out or "").lower()
    if rc == 127 or any(m in low for m in _NO_DEVICE_MARKERS):
        return GPU_NONE, []
    if rc != 0:
        return GPU_UNAVAILABLE, []
    gpus = parse_gpus(out)
    if not gpus:
        return GPU_NONE if not low.strip() else GPU_UNAVAILABLE, []
    return GPU_OK, gpus


# ── CPU ───────────────────────────────────────────────────────────────────────────────────────────────────────────
def parse_proc_stat(text):
    """The aggregate `cpu` line of /proc/stat -> (idle, total) in jiffies, or None.

    idle counts iowait as idle (the CPU had nothing to run); total is the first eight fields (user nice system idle
    iowait irq softirq steal). guest time is already inside user, so it is not added again.
    """
    for line in (text or "").splitlines():
        parts = line.split()
        if parts and parts[0] == "cpu":
            vals = [int(x) for x in parts[1:9] if x.isdigit()]
            if len(vals) < 4:
                return None
            idle = vals[3] + (vals[4] if len(vals) > 4 else 0)
            return idle, sum(vals)
    return None


def windows_times(idle, kernel, user):
    """GetSystemTimes' three counters -> (idle, total). ⚠ Windows' kernel time INCLUDES idle time, so the total is
    kernel + user, not idle + kernel + user (which would read every CPU as half idle)."""
    return idle, kernel + user


def cpu_percent(before, after):
    """Busy share of the CPU between two (idle, total) samples, 0-100, or None when it cannot be said."""
    if not before or not after:
        return None
    d_total = after[1] - before[1]
    d_idle = after[0] - before[0]
    if d_total <= 0 or d_idle < 0:
        return None
    return max(0.0, min(100.0, 100.0 * (d_total - d_idle) / d_total))


def parse_meminfo(text):
    """/proc/meminfo -> (used_mb, total_mb), used = MemTotal - MemAvailable, or None."""
    vals = {}
    for line in (text or "").splitlines():
        k, _, rest = line.partition(":")
        bits = rest.split()
        if bits and bits[0].isdigit():
            vals[k.strip()] = int(bits[0])
    if "MemTotal" not in vals or "MemAvailable" not in vals:
        return None
    total = vals["MemTotal"] / 1024.0
    return total - vals["MemAvailable"] / 1024.0, total


def read_cpu_times():
    """(idle, total) for the whole machine now, or None. Not pure: reads the OS."""
    try:
        if sys.platform == "win32":
            ft = [ctypes.c_ulonglong() for _ in range(3)]      # FILETIME is two DWORDs; one 64-bit read is the same
            if not ctypes.windll.kernel32.GetSystemTimes(*(ctypes.byref(x) for x in ft)):
                return None
            return windows_times(*(x.value for x in ft))
        with open("/proc/stat", encoding="ascii") as f:
            return parse_proc_stat(f.read())
    except Exception:          # noqa: BLE001 - a panel must never break proving
        return None


class _MEMORYSTATUSEX(ctypes.Structure):
    _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]


def read_memory():
    """(used_mb, total_mb) of system RAM now, or None. Not pure: reads the OS."""
    try:
        if sys.platform == "win32":
            m = _MEMORYSTATUSEX()
            m.dwLength = ctypes.sizeof(_MEMORYSTATUSEX)
            if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(m)):
                return None
            mb = 1024.0 * 1024.0
            return (m.ullTotalPhys - m.ullAvailPhys) / mb, m.ullTotalPhys / mb
        with open("/proc/meminfo", encoding="ascii") as f:
            return parse_meminfo(f.read())
    except Exception:          # noqa: BLE001
        return None


# ── one sample ────────────────────────────────────────────────────────────────────────────────────────────────────
class Sampler:
    """Takes one sample of everything per call. Runs on a background thread; touches no widget.

    `run(cmd, timeout=) -> (rc, out)` is supervisor._run in the app (never raises; 127 = not found), and a fake in
    the tests. `smi()` names the nvidia-smi to run. The CPU and memory readers are injectable for the same reason.
    """

    def __init__(self, run, smi=lambda: "nvidia-smi", cpu=read_cpu_times, mem=read_memory):
        self.run, self.smi, self.cpu, self.mem = run, smi, cpu, mem
        self._cpu_prev = None
        self.gpu_state = None
        self._skip = 0           # polls left before nvidia-smi is asked again after a `none` / `unavailable`

    def sample(self):
        out = {"gpu_state": self.gpu_state, "gpus": []}
        if self._skip > 0:
            self._skip -= 1
        else:
            rc, text = self.run([self.smi(), f"--query-gpu={GPU_QUERY}", "--format=csv,noheader,nounits"],
                                timeout=SMI_TIMEOUT_S)
            state, gpus = classify_smi(rc, text)
            self.gpu_state = out["gpu_state"] = state
            out["gpus"] = gpus
            out["gpu_why"] = "" if state == GPU_OK else (text or "").strip()[:200]
            self._skip = {GPU_NONE: RETRY_NONE_POLLS, GPU_UNAVAILABLE: RETRY_UNAVAILABLE_POLLS}.get(state, 0)
        now = self.cpu()
        out["cpu"] = cpu_percent(self._cpu_prev, now)
        self._cpu_prev = now or self._cpu_prev
        m = self.mem()
        out["ram_used_mb"], out["ram_total_mb"] = m if m else (None, None)
        return out


# ── which card is a worker's ──────────────────────────────────────────────────────────────────────────────────────
def gpu_for(card, gpus):
    """The sample row for a worker's card (by UUID, then by index), or None.

    A worker given no card (a one-card machine, where nothing is pinned) proves on whatever CUDA picks, so it is
    matched to the card using the most memory — on a one-card machine, that card.
    """
    if not gpus:
        return None
    if card:
        for g in gpus:
            if card.get("uuid") and g["uuid"] == card["uuid"]:
                return g
        for g in gpus:
            if g["index"] == card.get("index"):
                return g
        return None
    return max(gpus, key=lambda g: g["used_mb"] or 0)


def shown_gpus(gpus, pinned):
    """The cards the panel shows: those the workers are pinned to (in nvidia-smi's order), or all when none is."""
    if not pinned:
        return list(gpus)
    rows = [g for g in gpus if any(gpu_for(c, [g]) is g for c in pinned)]
    return rows or list(gpus)


# ── the peak of each block ────────────────────────────────────────────────────────────────────────────────────────
class BlockPeaks:
    """The highest GPU memory seen while each worker holds a block. Thread-safe: the reader thread starts and ends
    blocks (it sees the worker's lines first), the main thread feeds samples in.

        start(key)              a new block: the peak goes back to nothing
        observe(key, used, tot) one sample of that worker's card, while a block is held
        note(key, po2)          the log text for where the block finishes or fails — ONCE per block, then None
    """

    def __init__(self):
        self._lock = threading.Lock()
        self._b = {}

    def start(self, key):
        with self._lock:
            self._b[key] = {"peak": None, "total": None, "noted": False}

    def observe(self, key, used_mb, total_mb):
        if used_mb is None:
            return
        with self._lock:
            b = self._b.get(key)
            if b is None or b["noted"]:
                return                # no block in hand, or it has already finished: nothing to attribute this to
            if b["peak"] is None or used_mb > b["peak"]:
                b["peak"] = used_mb
            if total_mb:
                b["total"] = total_mb

    def peak(self, key):
        """(peak_mb, total_mb) of the block in hand, or (None, None)."""
        with self._lock:
            b = self._b.get(key)
            return (b["peak"], b["total"]) if b else (None, None)

    def note(self, key, po2=None):
        with self._lock:
            b = self._b.get(key)
            if b is None or b["noted"]:
                return None
            b["noted"] = True
            return peak_text(b["peak"], b["total"], po2)


def gb(mb):
    return f"{mb / 1024.0:.1f}"


def peak_text(peak_mb, total_mb, po2=None):
    """"peak GPU memory 3.6 of 4.0 GB at HAZYNC_SEG_PO2=18 (sampled every 2 s)" — or why there is no number."""
    at = f" at HAZYNC_SEG_PO2={po2}" if po2 not in (None, "") else ""
    if peak_mb is None:
        return f"peak GPU memory not sampled{at}"
    of = f" of {gb(total_mb)}" if total_mb else ""
    return f"peak GPU memory {gb(peak_mb)}{of} GB{at} (sampled every {POLL_S:g} s)"


# ── the panel's words and the chart ───────────────────────────────────────────────────────────────────────────────
def frac(used, total):
    """used/total clamped to 0-1, or None."""
    if used is None or not total:
        return None
    return max(0.0, min(1.0, used / total))


def gpu_line(g):
    """"3.4 / 4.0 GB · 97% busy · 71 °C" — a field nvidia-smi could not read is left out, not shown as 0."""
    bits = []
    if g.get("used_mb") is not None and g.get("total_mb"):
        bits.append(f"{gb(g['used_mb'])} / {gb(g['total_mb'])} GB")
    if g.get("util") is not None:
        bits.append(f"{g['util']:.0f}% busy")
    if g.get("temp") is not None:
        bits.append(f"{g['temp']:.0f} °C")
    return " · ".join(bits) or "no readings"


class History:
    """The last HISTORY_N samples of (GPU memory fraction, CPU fraction), either None when it was not read."""

    def __init__(self, n=HISTORY_N):
        self.gpu = deque(maxlen=n)
        self.cpu = deque(maxlen=n)

    def add(self, gpu_frac, cpu_pct):
        self.gpu.append(gpu_frac)
        self.cpu.append(None if cpu_pct is None else cpu_pct / 100.0)


def spark(values, width, height, n=HISTORY_N, pad=2):
    """Canvas line segments for a sparkline of 0-1 values, newest at the right edge. -> [[x0, y0, x1, y1, ...], ...]

    ⚠ A missing value (None) BREAKS the line rather than drawing it at zero: a gap is honest, a dip to nothing is a
    reading that never happened. A run of one point gets a tiny segment so it is still visible.
    """
    vals = list(values)[-n:]
    if not vals or width <= 2 * pad or height <= 2 * pad:
        return []
    step = (width - 2 * pad) / max(1, n - 1)
    x0 = width - pad - step * (len(vals) - 1)
    segs, cur = [], []
    for i, v in enumerate(vals):
        if v is None:
            if cur:
                segs.append(cur)
            cur = []
            continue
        v = max(0.0, min(1.0, v))
        cur += [round(x0 + i * step, 1), round(height - pad - v * (height - 2 * pad), 1)]
    if cur:
        segs.append(cur)
    return [s if len(s) >= 4 else s + [s[0] + 1, s[1]] for s in segs]


if __name__ == "__main__":
    # a quick look at this machine: `python stats.py`
    import subprocess

    def _run(cmd, timeout):
        try:
            p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
            return p.returncode, p.stdout + p.stderr
        except FileNotFoundError:
            return 127, "not found"
        except subprocess.TimeoutExpired:
            return -1, "timed out"
    s = Sampler(_run)
    s.sample()
    import time
    time.sleep(1)
    print(s.sample())
