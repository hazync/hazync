#!/usr/bin/env python3
"""The live resource panel's numbers (stats.py), tested without a graphics card or a display.

    python3 test_stats.py             # must PASS
    python3 test_stats.py --control   # "[N/A]" read as 0, Windows CPU total double-counting idle, a peak that never
                                      # resets between blocks, and a missing nvidia-smi read as "unavailable": MUST FAIL

⚠ WHAT THIS CANNOT SHOW. That GetSystemTimes and GlobalMemoryStatusEx answer through ctypes on a real Windows machine,
or what nvidia-smi prints on the owner's GTX 1050 Ti; the readers for those are thin and are exercised on Windows only.
The Linux readers (/proc) are run for real below, as a positive check that the same code path returns numbers.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import sys  # noqa: E402

import stats  # noqa: E402

CONTROL = "--control" in sys.argv
if CONTROL:
    print("CONTROL: the panel's maths as it would be if written carelessly -- the checks below MUST fail")
    stats._num = lambda f: int(f) if (f or "").strip().isdigit() else 0
    stats.windows_times = lambda idle, kernel, user: (idle, idle + kernel + user)
    stats.BlockPeaks.start = lambda self, key: (self._b.setdefault(key, {"peak": None, "total": None,
                                                                         "noted": False}),
                                                self._b[key].__setitem__("noted", False))
    stats.classify_smi = lambda rc, out: ((stats.GPU_OK, stats.parse_gpus(out)) if rc == 0
                                          else (stats.GPU_UNAVAILABLE, []))

fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


# ── nvidia-smi's CSV (--query-gpu=GPU_QUERY --format=csv,noheader,nounits) ──────────────────────────────────────────
# The shape nvidia-smi prints with nounits: comma-space separated, "[N/A]" where a field cannot be read.
ONE = "0, GPU-3a1f0c52-0000-0000-0000-000000000001, 3482, 4096, 97, 71\n"
TWO = ("0, GPU-aaaa, 9120, 10240, 88, 66\n"
       "1, GPU-bbbb, 512, 10240, 0, 41\n")
NA = "0, GPU-cccc, 1200, 4096, [N/A], [N/A]\n"

g = stats.parse_gpus(ONE)
check(len(g) == 1 and g[0]["index"] == 0 and g[0]["uuid"].startswith("GPU-3a1f"), "one card: index and uuid read")
check(g and (g[0]["used_mb"], g[0]["total_mb"], g[0]["util"], g[0]["temp"]) == (3482, 4096, 97, 71),
      "one card: used, total, utilisation and temperature read as numbers")
g2 = stats.parse_gpus(TWO)
check([x["index"] for x in g2] == [0, 1] and g2[1]["used_mb"] == 512, "two cards: both read, in nvidia-smi's order")
gn = stats.parse_gpus(NA)
check(gn and gn[0]["util"] is None and gn[0]["temp"] is None,
      "⛔ '[N/A]' is None, never 0 (a card at 0 °C would be a made-up reading)")
check(gn and gn[0]["used_mb"] == 1200, "...and the fields that WERE read still are")
check(stats.gpu_line(gn[0]) == "1.2 / 4.0 GB" if gn else False, "an [N/A] field is left out of the panel's line")
check(stats.gpu_line(g[0]) == "3.4 / 4.0 GB · 97% busy · 71 °C", "the panel's line for a full reading")
check(stats.parse_gpus("garbage\n, , ,\n") == [] and stats.parse_gpus("") == [], "unreadable lines are skipped")
check(stats.parse_gpus("0, GPU-x, 1, 2\n") == [], "a short line (old nvidia-smi, fewer fields) is skipped")

# ── no card / no nvidia-smi / a failing nvidia-smi ──────────────────────────────────────────────────────────────────
check(stats.classify_smi(127, "not found: nvidia-smi")[0] == stats.GPU_NONE,
      "nvidia-smi not found: 'CPU build — no GPU', not 'unavailable'")
check(stats.classify_smi(6, "No devices were found\n")[0] == stats.GPU_NONE, "no NVIDIA card: no GPU")
check(stats.classify_smi(-1, "timed out after 8s: nvidia-smi ...")[0] == stats.GPU_UNAVAILABLE,
      "a slow nvidia-smi: unavailable")
check(stats.classify_smi(9, "NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver")[0]
      == stats.GPU_UNAVAILABLE, "a driver nvidia-smi cannot talk to: unavailable (not claimed to be 'no GPU')")
check(stats.classify_smi(0, ONE) == (stats.GPU_OK, stats.parse_gpus(ONE)), "a good answer: ok, with the cards")


class FakeRun:
    def __init__(self, answers):
        self.answers, self.calls = list(answers), 0

    def __call__(self, cmd, timeout=None):
        self.calls += 1
        self.cmd, self.timeout = cmd, timeout
        return self.answers.pop(0) if self.answers else (127, "not found")


cpu_seq = iter([(100, 1000), (150, 1100), (150, 1100)])
run = FakeRun([(127, "not found: nvidia-smi")])
smp = stats.Sampler(run, cpu=lambda: next(cpu_seq), mem=lambda: (6000.0, 16000.0))
s1 = smp.sample()
check(s1["gpu_state"] == stats.GPU_NONE and s1["gpus"] == [], "the no-GPU path: state none, no cards")
check(s1["cpu"] is None, "the first sample has no CPU % yet (it needs two)")
check((s1["ram_used_mb"], s1["ram_total_mb"]) == (6000.0, 16000.0), "...but memory is read on the no-GPU path")
check("--format=csv,noheader,nounits" in run.cmd and f"--query-gpu={stats.GPU_QUERY}" in run.cmd
      and run.timeout == stats.SMI_TIMEOUT_S, "nvidia-smi is asked the panel's query, with a timeout")
s2 = smp.sample()
check(s2["gpu_state"] == stats.GPU_NONE and run.calls == 1,
      "no GPU: nvidia-smi is not run again on the next poll (it backs off)")
check(s2["cpu"] is not None and abs(s2["cpu"] - 50.0) < 1e-9, "the CPU half keeps going without a GPU (50%)")
smp._skip = 0                      # as if RETRY_NONE_POLLS polls had passed
run.answers = [(0, ONE)]
s3 = smp.sample()
check(s3["gpu_state"] == stats.GPU_OK and len(s3["gpus"]) == 1, "...and asks again later, and a card appearing shows")
smp2 = stats.Sampler(FakeRun([(-1, "timed out after 8s")]), cpu=lambda: None, mem=lambda: None)
s4 = smp2.sample()
check(s4["gpu_state"] == stats.GPU_UNAVAILABLE and "timed out" in s4["gpu_why"],
      "a slow nvidia-smi: unavailable, with its own words kept for the one log line")
check(s4["cpu"] is None and s4["ram_used_mb"] is None, "readers that return nothing give None, not a crash")

# ── CPU % from two samples ──────────────────────────────────────────────────────────────────────────────────────────
check(stats.cpu_percent((100, 1000), (150, 1100)) == 50.0, "50 idle of 100 ticks: 50% busy")
check(stats.cpu_percent((100, 1000), (200, 1100)) == 0.0, "all idle: 0%")
check(stats.cpu_percent((100, 1000), (100, 1100)) == 100.0, "never idle: 100%")
check(stats.cpu_percent((100, 1000), (100, 1000)) is None, "no time passed: None, not a division by zero")
check(stats.cpu_percent(None, (1, 2)) is None, "one sample only: None")
check(stats.cpu_percent((100, 1000), (90, 1100)) is None, "a counter that went backwards: None, not a number")
# Windows: kernel time includes idle. 4 s of wall time on 1 core: idle 1 s, kernel 2 s (1 idle + 1 busy), user 2 s
a = stats.windows_times(10_000_000, 20_000_000, 20_000_000)
b = stats.windows_times(20_000_000, 40_000_000, 40_000_000)
check(abs((stats.cpu_percent(a, b) or 0) - 75.0) < 1e-9,
      "⛔ Windows: total = kernel + user (kernel INCLUDES idle) — 1 s idle of 4 s is 75% busy")
STAT = ("cpu  4705 150 1120 16250 520 0 32 0 0 0\n"
        "cpu0 2000 70 500 8000 260 0 16 0 0 0\n")
check(stats.parse_proc_stat(STAT) == (16250 + 520, 4705 + 150 + 1120 + 16250 + 520 + 0 + 32 + 0),
      "/proc/stat: idle counts iowait; total is the first eight fields; the per-core lines are ignored")
check(stats.parse_proc_stat("intr 1 2 3\n") is None, "/proc/stat without a cpu line: None")
MEMINFO = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    6144000 kB\n"
check(stats.parse_meminfo(MEMINFO) == ((16384000 - 6144000) / 1024.0, 16384000 / 1024.0),
      "/proc/meminfo: used = total - available")
check(stats.parse_meminfo("MemTotal: 1 kB\n") is None, "/proc/meminfo without MemAvailable: None")

# ── the peak GPU memory of each block ───────────────────────────────────────────────────────────────────────────────
pk = stats.BlockPeaks()
pk.observe("w0", 3000, 4096)
check(pk.peak("w0") == (None, None), "a sample before any block is claimed belongs to no block")
pk.start("w0")
for used in (1200, 3600, 2100):
    pk.observe("w0", used, 4096)
pk.observe("w0", None, 4096)
check(pk.peak("w0") == (3600, 4096), "the peak is the highest sample of the block; an unread sample is skipped")
n1 = pk.note("w0", "18")
check(n1 == "peak GPU memory 3.5 of 4.0 GB at HAZYNC_SEG_PO2=18 (sampled every 2 s)",
      f"the log note names the peak, the card's total and the segment size ({n1!r})")
check(pk.note("w0", "18") is None, "the note is written ONCE per block (several failure lines, one note)")
pk.observe("w0", 4000, 4096)
check(pk.peak("w0") == (3600, 4096), "after the block has finished, a sample no longer moves its peak")
pk.start("w0")
pk.observe("w0", 800, 4096)
check(pk.peak("w0") == (800, 4096), "⛔ a NEW block starts from nothing: last block's 3.6 GB is not carried over")
check(pk.note("w0") == "peak GPU memory 0.8 of 4.0 GB (sampled every 2 s)",
      "...its own note, without a segment size when none was set")
pk.start("w1")
pk.observe("w1", 9000, 10240)
check(pk.peak("w0") == (800, 4096) and pk.peak("w1") == (9000, 10240), "two workers keep two peaks")
pk.start("w2")
check(pk.note("w2", "17") == "peak GPU memory not sampled at HAZYNC_SEG_PO2=17",
      "a block that ended before any sample says so rather than inventing a number")
check(pk.note("never") is None, "no block in hand: no note")

# ── which card is a worker's ────────────────────────────────────────────────────────────────────────────────────────
gs = stats.parse_gpus(TWO)
check(stats.gpu_for({"index": 1, "uuid": "GPU-bbbb"}, gs)["index"] == 1, "a pinned worker: its card, by uuid")
check(stats.gpu_for({"index": 1, "uuid": ""}, gs)["index"] == 1, "...or by index when there is no uuid")
check(stats.gpu_for({"index": 7, "uuid": "GPU-zzzz"}, gs) is None, "a card not in the sample: None, not a guess")
check(stats.gpu_for(None, stats.parse_gpus(ONE))["index"] == 0, "an unpinned worker on a one-card machine: that card")
check(stats.gpu_for(None, []) is None, "no cards: None")
check([g["index"] for g in stats.shown_gpus(gs, [{"index": 1, "uuid": "GPU-bbbb"}])] == [1],
      "the panel shows the card the workers are pinned to")
check([g["index"] for g in stats.shown_gpus(gs, [])] == [0, 1], "...and every card when nothing is pinned")

# ── the chart ───────────────────────────────────────────────────────────────────────────────────────────────────────
h = stats.History(n=4)
for v in (0.1, 0.2, 0.3, 0.4, 0.5):
    h.add(v, v * 100)
check(list(h.gpu) == [0.2, 0.3, 0.4, 0.5] and abs(h.cpu[-1] - 0.5) < 1e-9,
      "the history keeps the last N samples (five minutes at the poll rate), CPU as a share")
check(stats.HISTORY_N == 150, "five minutes at one sample every 2 s is 150 samples")
segs = stats.spark([0.0, 1.0], 104, 24, n=2, pad=2)
check(segs == [[2.0, 22.0, 102.0, 2.0]], f"0 at the bottom, 1 at the top, newest at the right edge ({segs})")
segs = stats.spark([0.5, None, 0.5, 0.5], 100, 20, n=4)
check(len(segs) == 2, "⚠ a missing reading breaks the line instead of dropping it to zero")
check(len(segs[0]) == 4, "a lone point still gets a visible stub")
check(stats.spark([], 100, 20) == [] and stats.spark([0.5], 2, 2) == [], "nothing to draw, or no room: nothing")
segs = stats.spark([0.5], 100, 20, n=150)
check(segs and segs[0][0] > 97, "a short history sits at the right edge, filling leftwards as it grows")

# ── positive control: the real readers on this machine return numbers (Linux /proc; Windows ctypes) ───────────────
if not CONTROL and (sys.platform == "win32" or _os.path.exists("/proc/stat")):
    t = stats.read_cpu_times()
    m = stats.read_memory()
    check(t is not None and t[1] > t[0] >= 0, f"this machine's CPU counters read ({t})")
    check(m is not None and 0 < m[0] < m[1], f"this machine's memory reads ({m})")

print()
if CONTROL:
    # four separate faults were planted, so at least one check per fault must catch it
    if fails >= 4:
        print(f"CONTROL OK: {fails} checks fail with the careless maths, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} failed")
    sys.exit(1)
if fails:
    print(f"FAIL {fails}")
    sys.exit(1)
print("stats: parsing, CPU %, peaks per block and the no-GPU path all hold")
