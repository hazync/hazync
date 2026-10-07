"""The logic behind the Windows GUI, with no tkinter in it — so it can be TESTED.

⛔ WHY THE SPLIT. A supervisor whose only entry point is a window cannot be tested on a build box,
and this project's standing lesson is that a check which cannot run is a check that does not exist.
Everything here is importable and headless; `hazync_gui.py` is a thin shell over it.

## What this replaces

A contributor on Linux runs three things: the `host` binary, the `hazync-worker` Python CLI, and
`run-workers.sh`, which is a supervisor loop — start N workers, restart them on transient failure,
stop them on a configuration failure. The GUI replaces the THIRD of those. The host binary and the
worker are used exactly as they ship, which matters because `hazync-worker` is a release asset whose
source is not in this repository: a forked worker would drift from the one the fleet runs.

## The four Windows gaps, and which are fixable from out here

    import fcntl              ⛔ module-level, so the worker dies at startup   -> winshim/fcntl.py
    /tmp/hazync-gpu.lock      POSIX path                                      -> HAZYNC_GPU_LOCK
    hazync-host-x86_64-...    hardcoded Linux binary names                    -> HAZYNC_HOST
    os.killpg(..., SIGKILL)   no process groups on Windows                    -> NOT fixable here

⚠ The last one is honest residual risk: it sits on the stall/timeout path, so a worker that hangs
will raise AttributeError instead of killing its child cleanly. That needs the worker's source. It
is recorded in the README rather than papered over.

## ⛔ One worker by default on Windows, and the reason is not caution

`gpu_lock()` serialises GPU work across workers on a box. With the shim it is a REAL lock, so N
workers are safe — but if the shim is ever absent or fails, the worker's own fallback is "proceed
rather than refuse to work", and N workers would then share one GPU with no serialisation at all. On
a 4 GB card that converts "slow" into "out of memory". ⇒ Default 1, and only raise it once a run has
shown the lock working.
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()   # ⛔ BEFORE anything prints: a ✅ on a cp1252 console raises, not degrades

import hashlib
import json
import os
import re
import struct
import subprocess
import time
import threading
import sys
import tempfile
from pathlib import Path

IS_WINDOWS = os.name == "nt"

# The canonical guest image id. A binary printing anything else produces proofs the coordinator
# rejects with EX_CONFIG, so this is the first thing checked and the only one that is fatal.
CANONICAL_METHOD_ID = "37987b85ec665970ac6c5e8031deb8160ac8ed846f09056c3790b5f78c8bb5dd"

# ⭐ The guest is embedded VERBATIM in the host binary, so "is this canonical" can be answered from
# the FILE — which matters because a CUDA build on a machine with no NVIDIA driver cannot start at
# all, and "it would not run" says nothing about whether it is genuine.
# ⛔ THE GUEST IS NOT AN ELF. It is risc0's R0BF container, despite being named *.elf everywhere.
# Searching for \x7fELF finds 51 coincidental hits in this binary and none of them is the guest.
# ⛔ And it is not the FIRST R0BF hit either — measured, it was the second.
GUEST_MAGIC = b"R0BF"
GUEST_SIZE = 69_980_208
GUEST_SHA256 = "35e3f55ed873de27f3e06b4452fb9ac80940e02f845b671a0e019cf83735c02f"

EX_CONFIG = 78      # the worker established it can never land anything — STOP, do not retry
EX_TEMPFAIL = 75    # nothing to claim right now — a busy board, not a fault

HEX64 = re.compile(r"\b[0-9a-f]{64}\b")


class CheckResult:
    """One named check with a verdict, its evidence, and whether it blocks starting."""

    def __init__(self, name, ok, detail, fatal=False):
        self.name, self.ok, self.detail, self.fatal = name, ok, detail, fatal

    def __repr__(self):
        return f"<{'ok' if self.ok else 'FAIL'} {self.name}: {self.detail}>"


def run_stream(cmd, env=None, timeout=60, on_line=None):
    """Run a command, delivering each output line to `on_line` AS IT ARRIVES. (rc, full output)

    ⛔⛔ WHY THIS EXISTS. _run uses subprocess.run(capture_output=True), which hands back everything
    only when the process EXITS. For a 2,875 s prove that means forty minutes of a window saying
    "running…" and nothing else — reported from a real machine as "there isn't any form of progress
    being shown". The prover is not silent at all; it prints

        execution 10.1 s   3 segments, 0.9 MB (streamed)
          0/2 segments  10s elapsed, ~0s left
          1/2 segments  16s elapsed, ~16s left

    and every line of that was being buffered away. A long job that shows nothing is
    indistinguishable from a hung one, and the honest reading of a frozen window is "it broke".

    ⚠ THE TIMEOUT IS A WATCHDOG THREAD, NOT A CHECK IN THE LOOP. Checking the clock as each line
    arrives cannot fire when the process goes SILENT, which is exactly the case a timeout is for.
    """
    buf = []
    try:
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, encoding="utf-8", errors="replace", env=env, bufsize=1)
    except FileNotFoundError:
        return 127, f"not found: {cmd[0]}"
    except OSError as e:
        return -1, f"could not run {cmd[0]}: {e}"

    killer = threading.Timer(timeout, p.kill)
    killer.daemon = True
    killer.start()
    try:
        for raw in p.stdout:
            buf.append(raw)
            if on_line:
                # ⚠ A progress line may be rewritten in place with \r rather than ended with \n,
                # so one read can carry several updates. Split them out instead of showing one
                # very long line that only ever grows.
                for part in raw.replace("\r\n", "\n").rstrip("\n").split("\r"):
                    if part.strip():
                        on_line(part.rstrip())
    except Exception as e:          # noqa: BLE001 - reporting a run must not raise
        buf.append(f"\n(reading output failed: {e})\n")
    finally:
        killer.cancel()
        try:
            p.stdout.close()
        except Exception:           # noqa: BLE001
            pass
    rc = p.wait()
    out = "".join(buf)
    if rc != 0 and not out.strip():
        out = f"(no output; exited {rc})"
    return rc, out


def _run(cmd, env=None, timeout=60):
    """(rc, combined output). Never raises — a missing binary is a result, not a crash."""
    try:
        # ⛔ encoding= MUST match what the child is told to emit. worker_env sets
        # PYTHONIOENCODING=utf-8; `text=True` alone decodes with the LOCALE encoding, which on
        # Windows is cp1252 -- so the pair would turn every non-ASCII character into mojibake and
        # make the fix look like a different bug. errors="replace" stays: a diagnostic must never
        # fail while reporting a failure.
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout,
                           env=env, encoding="utf-8", errors="replace")
        return p.returncode, (p.stdout or "") + (p.stderr or "")
    except FileNotFoundError:
        return 127, f"not found: {cmd[0]}"
    except subprocess.TimeoutExpired:
        return -1, f"timed out after {timeout}s: {' '.join(map(str, cmd))}"
    except OSError as e:
        return -1, f"could not run {cmd[0]}: {e}"


# ── what KIND of host binary is this? ────────────────────────────────────────────────────────────
#
# ⛔⛔ WHY THIS MATTERS MORE THAN IT LOOKS. A CUDA-linked host imports `nvcuda.dll`, the CUDA DRIVER
# api, which ships with the NVIDIA driver and NOT with the CUDA redist. On a machine with no NVIDIA
# driver, Windows cannot LOAD the image at all — so the exe does not run, and the failure says
# "command not found" or raises a module error, which reads as a missing or corrupt download.
#
# 📏 Measured 2026-10-04: the Windows CUDA host imports exactly 21 DLLs and `nvcuda.dll` is the only
# CUDA one — `cudart` is statically linked and does not appear. So the import table is the reliable
# way to tell the builds apart, and it can be read WITHOUT running anything.
#
# ⇒ Telling someone "this is the CUDA build and this machine has no NVIDIA driver" is a different
# and far more useful message than "the binary did not start".


def imported_dlls(path):
    """The DLLs a PE64 imports, read from the file. [] if it is not a PE64 or cannot be parsed."""
    try:
        data = Path(path).read_bytes()
    except OSError:
        return []
    if data[:2] != b"MZ":
        return []
    try:
        pe = struct.unpack_from("<I", data, 0x3C)[0]
        if data[pe:pe + 4] != b"PE\0\0":
            return []
        coff = pe + 4
        nsec, = struct.unpack_from("<H", data, coff + 2)
        opt_size, = struct.unpack_from("<H", data, coff + 16)
        opt = coff + 20
        if struct.unpack_from("<H", data, opt)[0] != 0x20B:      # PE32+ only
            return []
        imp_rva = struct.unpack_from("<I", data, opt + 112 + 8)[0]
        secs = []
        st = opt + opt_size
        for i in range(nsec):
            o = st + i * 40
            vsize, vaddr, rawsize, rawptr = struct.unpack_from("<IIII", data, o + 8)
            secs.append((vaddr, vsize, rawptr, rawsize))

        def off(rva):
            for vaddr, vsize, rawptr, rawsize in secs:
                if vaddr <= rva < vaddr + max(vsize, rawsize):
                    return rawptr + (rva - vaddr)
            return None

        base, out, i = off(imp_rva), [], 0
        if base is None:
            return []
        while True:
            f = struct.unpack_from("<IIIII", data, base + i * 20)
            if not any(f):
                break
            no = off(f[3])
            if no is None:
                break
            out.append(data[no:data.index(b"\0", no)].decode("latin1"))
            i += 1
        return out
    except (struct.error, ValueError, IndexError):
        return []


def classify_host(path):
    """What kind of prover is this, and can it start here? A dict, never an exception.

    Keys: kind ("cuda"/"cpu"/"unknown"), needs_driver, driver_present, dlls, size, startable.
    """
    dlls = [d.lower() for d in imported_dlls(path)]
    cuda = "nvcuda.dll" in dlls
    driver = driver_present()
    try:
        size = Path(path).stat().st_size
    except OSError:
        size = 0
    return {
        "kind": "cuda" if cuda else ("cpu" if dlls else "unknown"),
        "needs_driver": cuda,
        "driver_present": driver,
        "dlls": dlls,
        "size": size,
        # ⚠ "startable" is about LOADING the image, not about proving. A CPU build is startable
        # everywhere; a CUDA build needs the driver's nvcuda.dll present.
        "startable": (driver or not cuda) if dlls else None,
    }


def driver_present():
    """Is an NVIDIA driver installed? ⚠ Not 'is there a GPU' — nvcuda.dll is what loading needs."""
    if IS_WINDOWS:
        root = os.environ.get("SystemRoot", r"C:\Windows")
        if (Path(root) / "System32" / "nvcuda.dll").is_file():
            return True
    rc, _ = _run(["nvidia-smi", "-L"], timeout=20)
    return rc == 0


def host_kind_sentence(info):
    """One sentence a person can act on, from classify_host()."""
    if info["kind"] == "unknown":
        return "not a Windows executable this can read — is it the right file?"
    if info["kind"] == "cpu":
        return f"the CPU build ({info['size'] / 1e6:.0f} MB) — runs anywhere, slower, no GPU needed"
    if info["driver_present"]:
        return f"the CUDA build ({info['size'] / 1e6:.0f} MB) and an NVIDIA driver is present"
    return (f"the CUDA build ({info['size'] / 1e6:.0f} MB), but NO NVIDIA driver was found. It "
            f"imports nvcuda.dll, so Windows cannot even load it on this machine — that is not a "
            f"bad download. Use the CPU build here, or install the NVIDIA driver.")


def verify_embedded_guest(path):
    """(ok, detail) — is the canonical guest embedded in this binary? Reads the file, runs nothing.

    ⚠ This establishes the GUEST, not the image id. The id is computed from the guest, and no
    MSVC-linked build can recompute it (risc0-zkvm-platform's sys_alloc_aligned is an unresolved
    external under link.exe), which is exactly why the project carries a second pin for the guest's
    own sha256.
    """
    try:
        data = Path(path).read_bytes()
    except OSError as e:
        return False, f"cannot read it: {e}"
    start = 0
    hits = 0
    while True:
        i = data.find(GUEST_MAGIC, start)
        if i < 0:
            break
        start, hits = i + 1, hits + 1
        if i + GUEST_SIZE <= len(data):
            if hashlib.sha256(data[i:i + GUEST_SIZE]).hexdigest() == GUEST_SHA256:
                return True, f"canonical guest embedded at offset {i:,} ({GUEST_SIZE:,} bytes)"
    return False, (f"no embedded {GUEST_SIZE:,}-byte guest matched the pin "
                   f"({hits} R0BF marker(s) checked)")


def check_host_binary(host_path):
    """⛔ THE CHECK THAT MATTERS: does this binary carry the canonical METHOD_ID?

    ⚠ Parsed as "the first 64-hex token in the output", because `host method-id` prints the digest
    AND a `u32x8` decomposition, and a looser match would pick up the words. Measured output:

        METHOD_ID 37987b85ec665970ac6c5e8031deb8160ac8ed846f09056c3790b5f78c8bb5dd
          u32x8   [2239469623, ...]
    """
    host = Path(host_path)
    if not host.is_file():
        return CheckResult("host binary", False, f"no such file: {host}", fatal=True)
    info = classify_host(host)
    rc, out = _run([str(host), "method-id"], timeout=120)
    if rc != 0:
        # ⛔⛔ A BINARY THAT WILL NOT START IS NOT THE SAME AS A BAD BINARY, and conflating them sent
        # a real user looking for a corrupt download. A CUDA build imports nvcuda.dll from the
        # NVIDIA driver; with no driver, Windows cannot load the image at all.
        # ⇒ Fall back to reading the guest out of the FILE, and report the two facts separately.
        ok, detail = verify_embedded_guest(host)
        if info["kind"] == "cuda" and not info["driver_present"]:
            msg = ("this is the CUDA build and no NVIDIA driver was found, so Windows cannot load "
                   "it here. " + ("The file itself is genuine — " + detail + ". Use the CPU build "
                                  "on this machine, or install the NVIDIA driver."
                                  if ok else "And " + detail + "."))
            return CheckResult("host binary", False, msg, fatal=True)
        if ok:
            return CheckResult("host binary", False,
                               f"`method-id` exited {rc} so the id could not be read, but the file "
                               f"is genuine ({detail}). Output: {out.strip()[:120]}", fatal=True)
        return CheckResult("host binary", False,
                           f"`method-id` exited {rc} and {detail}. Output: {out.strip()[:140]}",
                           fatal=True)
    m = HEX64.search(out)
    if not m:
        return CheckResult("host binary", False, f"no image id in output: {out.strip()[:160]}",
                           fatal=True)
    got = m.group(0)
    if got != CANONICAL_METHOD_ID:
        return CheckResult("host binary", False,
                           f"METHOD_ID is {got}, NOT the canonical {CANONICAL_METHOD_ID}. "
                           f"Proofs from this binary would be rejected (exit {EX_CONFIG}).",
                           fatal=True)
    return CheckResult("host binary", True,
                       f"canonical METHOD_ID {got[:16]}… — {host_kind_sentence(info)}")


def gpu_facts():
    """What card is this, in numbers. (name, vram_mb, driver, cc_major, cc_minor) or None.

    ⭐ SEPARATED FROM check_gpu BECAUSE A DECISION NEEDS THE NUMBERS, NOT A SENTENCE. Choosing a
    prover and a segment size from a human-readable string means parsing prose, and that is how a
    setting ends up wrong on the one machine nobody tested.
    """
    rc, out = _run(["nvidia-smi", "--query-gpu=name,memory.total,driver_version,compute_cap",
                    "--format=csv,noheader"], timeout=30)
    if rc != 0 or not out.strip():
        return None
    parts = [x.strip() for x in out.strip().splitlines()[0].split(",")]
    if len(parts) < 4:
        return None
    mb = re.search(r"(\d+)", parts[1])
    cc = re.match(r"(\d+)\.(\d+)", parts[3])
    if not (mb and cc):
        return None
    return {"name": parts[0], "vram_mb": int(mb.group(1)), "driver": parts[2],
            "cc_major": int(cc.group(1)), "cc_minor": int(cc.group(2))}


# ⛔⛔ EVERY NUMBER HERE WAS MEASURED, NOT CHOSEN. The segment size decides peak VRAM, and the only
# hard data this project has is: one prove at HAZYNC_SEG_PO2=21 peaked near 22 GB. Each step down
# roughly halves the working set, so the table below is that one measurement walked down — which is
# honest extrapolation, and is labelled as such rather than presented as a measurement.
#
# ⚠ A CARD BELOW 8 GB HAS NEVER PROVED ANYTHING ON THIS PROJECT. The smallest that has is 24 GB.
# So for small cards this returns a STARTING POINT and says so; it does not promise it will work.
_SEG_PO2_FOR_VRAM = [(40000, 21), (20000, 20), (10000, 19), (6000, 18), (0, 17)]


def recommend(gpu=None):
    """What this machine should actually be set to. (dict) — the whole point of the Setup tab.

    ⛔ THE POINT IS THAT NOBODY SHOULD HAVE TO KNOW ANY OF THIS. Before this, a person had to learn
    that sppark rejects cards below compute 7.0, that there are two host binaries, that one of them
    cannot start without an NVIDIA driver, and that a segment size exists at all — by hitting each
    one as a failure. All four are answerable from `nvidia-smi` in under a second.
    """
    g = gpu if gpu is not None else gpu_facts()
    if not g:
        return {"build": "cpu", "seg_po2": None, "gpu": None,
                "why": "No NVIDIA driver was found, so a CUDA build could not even start. "
                       "The CPU build works anywhere."}
    seg = next(po2 for floor, po2 in _SEG_PO2_FOR_VRAM if g["vram_mb"] >= floor)
    if g["cc_major"] < 7:
        return {"build": "cpu", "seg_po2": None, "gpu": g,
                "why": f"{g['name']} is compute {g['cc_major']}.{g['cc_minor']}, and sppark keeps "
                       f"only cards at 7.0 or newer, so CUDA proving fails with \"no CUDA-capable "
                       f"device is detected\". The CPU build is the one that works here. "
                       f"(HAZYNC_SPPARK_MIN_MAJOR={g['cc_major']} lifts that floor on a build from "
                       f"2026-10-05 or later — unproven, see hazync#631.)"}
    note = ""
    if g["vram_mb"] < 8000:
        note = (f" ⚠ {g['vram_mb']:,} MiB is below anything this project has proved with — the "
                f"smallest measured is 24 GB — so {seg} is a starting point, not a promise.")
    return {"build": "cuda", "seg_po2": seg, "gpu": g,
            "why": f"{g['name']}, compute {g['cc_major']}.{g['cc_minor']}, {g['vram_mb']:,} MiB. "
                   f"Suggested HAZYNC_SEG_PO2={seg}, extrapolated from one measured prove that "
                   f"peaked near 22 GB at 21.{note}"}


def check_gpu():
    """GPU name, VRAM and the DRIVER's CUDA version — all three matter, and not just for display.

    📏 Measured 2026-10-04: a GTX 1050 Ti with 4 GB and driver CUDA 12.7 ran `method-id` and
    `regress` fine and then aborted on the first GPU proving call. VRAM and the driver's CUDA version
    are the two numbers that make that diagnosable, so they are surfaced rather than hidden.
    """
    rc, out = _run(["nvidia-smi", "--query-gpu=name,memory.total,driver_version,compute_cap",
                    "--format=csv,noheader"], timeout=30)
    if rc != 0:
        rc2, out2 = _run(["nvidia-smi"], timeout=30)
        if rc2 != 0:
            return CheckResult("GPU", False,
                               "nvidia-smi did not run. Without an NVIDIA driver a CUDA build "
                               "cannot start at all; the CPU build still works.")
        return CheckResult("GPU", True, out2.strip().splitlines()[0][:120])
    line = out.strip().splitlines()[0] if out.strip() else ""
    note = ""

    # ⛔⛔ COMPUTE CAPABILITY IS A HARD FLOOR, AND IT IS THE ANSWER TO hazync#631.
    #
    # 📏 Measured 2026-10-05 on a GTX 1050 Ti, with the build that finally prints the GPU's own
    # error: `cudaErrorNoDevice@sppark/util/all_gpus.cpp:43 — "no CUDA-capable device is detected"`,
    # on a machine where nvidia-smi lists the card and the driver is current.
    #
    # It is not a failing CUDA call. all_gpus.cpp enumerates devices and KEEPS ONLY THOSE WITH
    # `prop.major >= PROP_MAJOR_MIN`, and sppark sets that to 7 — "Volta and forward". A 1050 Ti is
    # Pascal, compute capability 6.1, so it is filtered out, the list comes back EMPTY, and
    # select_gpu raises a synthetic cudaErrorNoDevice. The card is fine; it is below sppark's floor.
    #
    # ⭐⭐ WHICH MEANS THIS WAS NEVER A WINDOWS BUG. The same card on Linux fails the same way. What
    # Windows added was only that the error was destroyed on its way out (#631's foreign-exception
    # abort), so a card-support question looked like a platform question for weeks.
    # ⚠ THE FLOOR CAN BE LOWERED, so this must not keep declaring failure once someone has done it.
    # A build from 2026-10-05 or later honours HAZYNC_SPPARK_MIN_MAJOR; reporting "use the CPU
    # build" to a person who has already overridden it would be telling them to undo the thing they
    # just did, which is how a check gets ignored.
    floor = 7
    override = os.environ.get("HAZYNC_SPPARK_MIN_MAJOR", "").strip()
    if override.isdigit() and 1 <= int(override) <= 99:
        floor = int(override)

    cc = re.search(r"(\d+)\.(\d+)\s*$", line)
    if cc and int(cc.group(1)) >= floor and floor < 7:
        return CheckResult(
            "GPU", True,
            line + f"  ⚠ accepted only because HAZYNC_SPPARK_MIN_MAJOR={floor} lowers sppark's "
                   "floor from 7. The kernels are built for sm_61, but whether Pascal computes "
                   "these proofs CORRECTLY is unverified — the receipt is checked before anything "
                   "is submitted, so a wrong result fails rather than reaching the board.")
    if cc and int(cc.group(1)) < 7:
        return CheckResult(
            "GPU", False,
            line + f"  ⛔ compute capability {cc.group(1)}.{cc.group(2)} — sppark keeps only "
                   "cards with major >= 7 (Volta or newer), so this one is filtered out and CUDA "
                   "proving fails with 'no CUDA-capable device is detected' however much VRAM it "
                   "has. Use the CPU build on this machine — or try a build from 2026-10-05 or "
                   "later with HAZYNC_SPPARK_MIN_MAJOR=6, which lowers sppark's floor (hazync#631).",
            fatal=False)

    mb = re.search(r"(\d+)\s*MiB", line)
    if mb and int(mb.group(1)) < 8000:
        # ⚠ Not presented as a failure: it has never been measured either way below 24 GB.
        note = ("  ⚠ under 8 GB. The smallest card measured to prove has 24 GB, and one prove at "
                "HAZYNC_SEG_PO2=21 peaked near 22 GB, so expect to lower HAZYNC_SEG_PO2.")
    return CheckResult("GPU", True, line + note)


def check_worker(worker_path, python_exe=None):
    """The worker must be present AND importable — on Windows that means the fcntl shim works."""
    w = Path(worker_path)
    if not w.is_file():
        return CheckResult("worker", False, f"no such file: {w}", fatal=True)
    py = python_exe or sys.executable
    # ⛔⛔ CHECK THE CONTENT FIRST. `rc in (0, 1)` was treated as "it ran", and a 14 KB HTML error
    # page run as Python exits 1 — so a failed download PASSED this check. A GitHub error page is a
    # perfectly valid file; only its contents say otherwise. Caught by a test that fed one in.
    try:
        head = w.read_bytes()[:4096]
    except OSError as e:
        return CheckResult("worker", False, f"cannot read {w}: {e}", fatal=True)
    if not head.startswith(b"#!") or b"import" not in head:
        return CheckResult("worker", False,
                           f"{w.name} is not the worker — it does not begin like a Python program "
                           f"({len(head)} bytes read, starts {head[:28]!r}). A failed download "
                           f"often leaves an HTML error page with a perfectly ordinary size.",
                           fatal=True)
    env = worker_env(host_path=None, worker_path=w, identity_dir=None, bundle_dir=None)
    rc, out = _run([py, str(w), "--help"], env=env, timeout=60)
    if rc != 0 and "No module named 'fcntl'" in out:
        return CheckResult("worker", False,
                           "the worker cannot import `fcntl` — the Windows shim is not on "
                           "PYTHONPATH. That is a module-level import, so the worker dies at "
                           "startup rather than degrading.", fatal=True)
    # ⚠ `--help` exits 0 and must PRINT its own commands. Checking the exit code alone is what let
    # the error page through; the output has to look like the worker's help.
    low = (out or "").lower()
    named = [c for c in ("run", "fold", "prove", "submit", "id") if c in low]
    if rc != 0 or len(named) < 3:
        # ⛔⛔ THE LAST LINES, NOT THE FIRST. A Python traceback puts the EXCEPTION at the END;
        # its first 160 characters are "Traceback (most recent call last)" and a file path. That is
        # exactly what was shown on Windows 2026-10-05 -- a person saw the header of the error and
        # not the error, and the one line that named the cause was cut off.
        tail = "\n".join((out or "").strip().splitlines()[-6:])
        return CheckResult("worker", False,
                           f"`--help` exited {rc} and did not print the worker's commands "
                           f"(recognised {named}). How it ended:\n{tail}", fatal=True)
    return CheckResult("worker", True, f"{w.name} starts and its CLI lists {len(named)} commands")


def check_signing_library(python_exe=None):
    """The worker signs its claims; without `cryptography` it exits before doing any work."""
    py = python_exe or sys.executable
    rc, out = _run([py, "-c", "import cryptography; print(cryptography.__version__)"], timeout=60)
    if rc != 0:
        return CheckResult("signing library", False,
                           "`cryptography` is not installed for this Python. The worker signs "
                           "every claim, so it cannot run without it:  pip install cryptography",
                           fatal=True)
    return CheckResult("signing library", True, f"cryptography {out.strip()}")


def gpu_lock_path():
    """Where the cross-worker GPU lock lives. ⚠ The worker's default is a POSIX /tmp path."""
    return str(Path(tempfile.gettempdir()) / "hazync-gpu.lock")


def shim_dir():
    return str(Path(__file__).resolve().parent / "winshim")


def worker_env(host_path, worker_path, identity_dir, bundle_dir, coord_url=None,
               seg_po2=None, base_env=None, force_shim=None):
    """The environment one worker runs with.

    ⚠ THE SHIM GOES ON PYTHONPATH ON WINDOWS ONLY — as cheap defence, not because shadowing is
    proven. MEASURED on the dev box: `fcntl` is a BUILT-IN module there (`fcntl.__file__` is
    `<built-in>`), so `BuiltinImporter` resolves it from sys.meta_path before the path finder and the
    shim cannot shadow it. ⚠ But that is a property of the Python BUILD: where `fcntl` ships as a
    shared extension in lib-dynload it is found via sys.path and the shim WOULD win, replacing real
    `flock` with one that refuses and silently ending the fleet's GPU serialisation. The gate costs
    nothing and removes the question; the shim also raises on POSIX rather than pretending, so even
    if it did load the mistake would be loud.
    """
    env = dict(base_env if base_env is not None else os.environ)
    use_shim = IS_WINDOWS if force_shim is None else force_shim
    if use_shim:
        existing = env.get("PYTHONPATH", "")
        env["PYTHONPATH"] = shim_dir() + (os.pathsep + existing if existing else "")
    if host_path:
        env["HAZYNC_HOST"] = str(host_path)
    if identity_dir:
        env["HAZYNC_HOME"] = str(identity_dir)
    if bundle_dir:
        env["BUNDLE_DIR"] = str(bundle_dir)
    if coord_url:
        env["COORD_URL"] = coord_url
    if seg_po2:
        env["HAZYNC_SEG_PO2"] = str(seg_po2)
    # ⛔⛔ THE WORKER CANNOT PRINT ITS OWN HELP ON WINDOWS WITHOUT THIS. Its docstring contains `⛔`
    # (U+26D4), Windows stdio defaults to cp1252, and cp1252 CANNOT ENCODE IT. So `hazync-worker
    # --help` dies with
    #
    #     UnicodeEncodeError: 'charmap' codec can't encode character '\u26d4' ... position 1295
    #
    # before doing anything. Measured on Windows 2026-10-05: the client downloaded correctly and was
    # then rejected as "it does not run", because the first thing check_worker asks it to do is
    # print that help. Reproduced on Linux with PYTHONIOENCODING=cp1252 -- rc=1 -- and fixed by this
    # line -- rc=0.
    #
    # ⚠ The worker is a RELEASE ASSET and is used exactly as it ships, so this has to be fixed from
    # the outside. It is set on every platform: on Linux the value is already utf-8, so it changes
    # nothing there and cannot drift into a Windows-only code path nobody exercises.
    env["PYTHONIOENCODING"] = "utf-8"
    # ⚠ Always set, because the worker's default is /tmp and Windows has no /tmp.
    env["HAZYNC_GPU_LOCK"] = gpu_lock_path()
    # Unbuffered, or the GUI's log pane shows nothing until a worker exits.
    env["PYTHONUNBUFFERED"] = "1"
    return env


def classify_exit(rc):
    """(kind, human sentence) for a worker exit. The three cases are NOT interchangeable.

    ⛔ `run-workers.sh` learned this the hard way: retrying EX_CONFIG meant three GPUs proving for a
    day into guaranteed rejection, looking busy the whole time. A supervisor that treats every
    non-zero exit as transient reproduces exactly that.
    """
    if rc == 0:
        return "done", "finished cleanly"
    if rc == EX_CONFIG:
        return "config", ("misconfigured — retrying CANNOT help. Usually the guest id does not "
                          "match the coordinator, so nothing this worker proves would be accepted. "
                          "The supervisor stops rather than burning the GPU.")
    if rc == EX_TEMPFAIL:
        return "idle", "nothing to claim right now — a busy board, not a fault. Waiting."
    return "transient", f"exited {rc} — treated as transient and retried"


def interesting_line(line):
    """Should this worker output line be surfaced prominently? (kind or None)

    ⭐ `CUDA ERROR:` exists because of #631: sppark's throw is a foreign exception on MSVC, so the
    process aborts before any Rust handler runs and the real CUDA error used to be destroyed. It is
    now printed just before the throw, and it is the single most useful line a Windows user can see.
    """
    s = line.strip()
    low = s.lower()
    if s.startswith("CUDA ERROR:"):
        return "cuda-error"
    if "fatal runtime error" in low and "foreign exception" in low:
        return "abort"
    if "out of memory" in low and not is_worker_hint(s):
        return "oom"
    if "proved" in low and "verified" in low:
        return "proved"
    if low.startswith("✓") or "the coordinator re-verified" in low:
        return "accepted"
    if "claimed block" in low:
        return "claimed"
    return None


# ── translating the failures we already know about ───────────────────────────────────────────────
#
# ⛔ WHY THIS IS NOT COSMETIC. Every one of these cost real time to diagnose, and a newcomer hitting
# the same string has none of that context. "fatal runtime error: Rust cannot catch foreign
# exceptions" tells a person nothing; "the GPU reported an error and the old build threw it away"
# tells them what to do next.
#
# ⚠ Each entry is a string that was actually OBSERVED, not one that seemed likely.
_EXPLANATIONS = [
    ("rust cannot catch foreign exceptions",
     "The GPU reported an error and this build discarded it. sppark throws a C++ exception, which "
     "on Windows aborts before any Rust handler runs. A build from 2026-10-04 or later prints a "
     "CUDA ERROR line just above this one — if there is none, you are on an older binary."),
    # ⚠ FALLBACK ONLY. explain() parses the CUDA line directly and quotes it, so this is reached
    # only if the line is formatted in a way _CUDA_ERROR_RE does not match. Kept so an unexpected
    # format still says something useful rather than falling through to the abort message.
    ("cuda error:",
     "This is the real GPU error, printed just before the abort. Read the text after the colon: "
     "'out of memory' means lower HAZYNC_SEG_PO2; anything else is the cause itself."),
    ("out of memory",
     "The GPU ran out of memory. Lower HAZYNC_SEG_PO2 on the Prove tab — each step down roughly "
     "halves it. One prove at 21 peaked near 22 GB on a 46 GB card, so a small card needs 18 or "
     "less, and may not manage at all."),
    ("no cuda-capable device is detected",
     "sppark keeps only GPUs with compute capability 7.0 or newer (Volta and forward), so an older "
     "card is filtered out and the device list comes back empty — this error is raised by sppark "
     "itself, not by the driver. nvidia-smi will still list the card. Use the CPU build. "
     "(hazync#631: a GTX 1050 Ti, compute 6.1, measured 2026-10-05.)"),
    ("no kernel image is available",
     "The binary has no code for this GPU. It is built for sm_61 plus forward PTX, so this usually "
     "means the driver is too old to compile the PTX rather than the card being wrong."),
    ("no module named 'fcntl'",
     "The worker could not start: fcntl is POSIX-only and the Windows stand-in was not on its "
     "path. That is a bug in this program, not in your setup — please report it."),
    ("could not find `protoc`",
     "A build-time dependency is missing. This should never appear in a released binary; it means "
     "the prover was built without protoc."),
    ("method_id", "The prover's guest id does not match the coordinator's, so every proof it makes "
                  "would be rejected. Download the prover again."),
    ("0xc000007b",
     "Windows could not load the executable — usually a missing DLL. A CUDA build needs the NVIDIA "
     "driver and cudart64_*.dll beside it."),
    ("claimed block",
     "A block is now yours for 60 minutes. If this machine cannot finish it, stop rather than "
     "retrying: abandoned claims hold up everyone else."),
]


# ⛔⛔ A REAL `CUDA ERROR:` LINE OUTRANKS EVERYTHING ELSE IN THE OUTPUT.
# The 2026-10-04 build prints the GPU's own error AND THEN still aborts, so both strings are present
# in the same output. First-match ordering explained the abort -- "if there is none, you are on an
# older binary" -- which is exactly backwards when the line IS there: it would have told the one
# person running the new build that they were running the old one, and thrown away the only
# evidence #631 has ever produced. Caught minutes before that run.
_CUDA_ERROR_RE = re.compile(r"^\s*CUDA ERROR:\s*(.+?)\s*$", re.M | re.I)


# ⛔⛔ THE WORKER'S OWN ADVICE CONTAINS THE WORDS WE MATCH ON. Every CUDA failure prints
#
#     (the abort that follows is this error crossing C++ -> Rust on MSVC;
#      if it mentions out of memory, retry with a lower HAZYNC_SEG_PO2)
#
# so a plain substring search for "out of memory" fires on EVERY failure, whatever caused it.
# ⚠ MEASURED 2026-10-07 on a real GTX 1050 Ti: a card failing with cudaErrorNotSupported was told
# "The GPU ran out of memory", and that sent an afternoon after VRAM on a 4 GB card when the error
# had nothing to do with memory. A newcomer would lower HAZYNC_SEG_PO2, fail again identically, and
# conclude the app is broken -- which is the single most expensive wrong answer this program can
# give, because it is confident and it is actionable.
_WORKER_HINT_MARKERS = ("if it mentions", "retry with a lower")


def is_worker_hint(line):
    """True for the worker's own boilerplate advice, which must never be read as EVIDENCE."""
    low = (line or "").lower()
    return all(m in low for m in _WORKER_HINT_MARKERS)


def without_worker_hints(text):
    """The output with the worker's advisory lines removed, for matching against."""
    return "\n".join(l for l in (text or "").splitlines() if not is_worker_hint(l))


def cuda_error_text(text):
    """The GPU's own error message, if this build printed one. (hazync#631)"""
    m = _CUDA_ERROR_RE.search(text or "")
    return m.group(1).strip() if m else None


def explain(text):
    """Plain language for a known failure, or None.

    ⚠ The CUDA line is checked FIRST and QUOTED, because it is the cause and everything after it is
    a consequence. Everything else matches in order and takes the first hit.
    """
    err = cuda_error_text(text)
    if err:
        low_err = err.lower()
        if "out of memory" in low_err:
            advice = ("The GPU ran out of memory. Lower HAZYNC_SEG_PO2 on the Prove tab — each step "
                      "down roughly halves it. A prove at 21 peaked near 22 GB, so a 4 GB card "
                      "needs far less and may not manage at all.")
        elif "no cuda-capable device" in low_err:
            # ⛔ The card is present and the driver is fine; sppark filtered it out. Saying "no
            # device" without that is how this looked like a broken driver for weeks.
            advice = ("sppark keeps only cards with compute capability 7.0 or newer (Volta and "
                      "forward) — all_gpus.cpp — so an older GPU is filtered out and the device "
                      "list comes back EMPTY. nvidia-smi will still list your card and the driver "
                      "is not at fault. Use the CPU build on the Setup tab — measured 2026-10-07, "
                      "folding on CPU runs about 13x slower than a modern GPU, which is slower but "
                      "still a real contribution. (hazync#631)")
        elif "operation not supported" in low_err:
            # ⛔ THE ERROR A PASCAL CARD ACTUALLY GETS ONCE THE FLOOR IS LOWERED, and the one that
            # used to fall through to "that is the cause" -- true, and useless to act on.
            # Measured 2026-10-07 on a GTX 1050 Ti: the card passes sppark's filter, and every
            # capability it needs tests FINE in isolation (plain and cooperative launches, grid
            # sync, memory pools, 1024-thread blocks, this very stream call) -- yet the prover
            # cannot drive it. Unresolved; tracked as hazync#631.
            advice = ("Your card was accepted but the prover could not drive it — hazync#631, seen "
                      "on Pascal (compute 6.x). This is not your setup and not your driver: the "
                      "same card passes every capability test on its own. Switch to the CPU build "
                      "on the Setup tab; it works, and folding measures about 13x slower than a "
                      "modern GPU — which still contributes.")
        elif "no kernel image" in low_err:
            advice = ("The binary has no code this GPU can run. Built for sm_61 plus forward PTX, "
                      "so this usually means the driver cannot compile the PTX.")
        else:
            advice = ("That is the cause. Everything printed after it is the abort that follows, "
                      "not a separate problem.")
        return f"the GPU reported: {err}\n  {advice}"
    # ⚠ Stripped FIRST: the worker's hint would otherwise match "out of memory" on every failure.
    low = without_worker_hints(text).lower()
    for needle, said in _EXPLANATIONS:
        if needle in low:
            return said
    return None


# ── the diagnostics a person would otherwise type by hand ────────────────────────────────────────
# (name, title, note, timeout, run_automatically)
#
# ⭐ THE LAST FIELD IS A COST DECISION, NOT A PREFERENCE. The first two answer in seconds and touch
# nothing outside this machine, so making a person find and press them is pure friction -- they ran
# them, in order, every time. The third takes 2,875 s on four CPU cores. Running that at startup
# would mean the window pins a machine for the better part of an hour before anyone asked it to,
# which is how a tool gets uninstalled.
#
# ⚠ So "should this be automatic" is answered per check, by what it costs, and the window says which
# is which rather than leaving someone to guess which button is safe to press.
DIAGNOSTICS = [
    ("method-id", "Is this prover genuine?",
     "Prints the guest id. Must be the canonical one or every proof is rejected.", 120, True),
    ("regress", "Does consensus work here?",
     "Replays block 170 through the full consensus path. Seconds, no GPU.", 300, True),
    ("prove-block", "Can it actually prove?",
     "Builds block 170's witness in-process and produces a real STARK receipt. This is the long "
     "one — 2875 s on four CPU cores; a GPU should be far quicker.", 10800, False),
]

AUTO_DIAGNOSTICS = [d[0] for d in DIAGNOSTICS if d[4]]


def diagnostic_command(host, name):
    return [str(host), name]


def diagnostic_verdict(name, rc, out):
    """(ok, sentence) for a finished diagnostic. Reads the OUTPUT, not just the exit code."""
    low = (out or "").lower()
    why = explain(out)
    if name == "method-id":
        m = HEX64.search(out or "")
        if m and m.group(0) == CANONICAL_METHOD_ID:
            return True, f"canonical: {m.group(0)[:16]}…"
        if m:
            return False, f"NOT canonical: {m.group(0)[:16]}… — proofs would be rejected"
        return False, (why or f"no id printed (exit {rc})")
    if name == "regress":
        # ⛔ Look for the PASS line, not for exit 0. A check that passes on silence is not a check.
        if "regression pass" in low:
            return True, "consensus regression passed"
        return False, (why or f"did not print a pass (exit {rc})")
    if name == "prove-block":
        if "proved" in low and ("verified" in low or "receipt" in low):
            import re as _re
            t = _re.search(r"PROVED in ([0-9.]+)s", out or "")
            return True, (f"proved and verified in {t.group(1)}s" if t else "proved and verified")
        return False, (why or f"did not produce a receipt (exit {rc})")
    return rc == 0, (why or f"exit {rc}")


def preflight(host_path, worker_path, identity_dir=None, python_exe=None):
    """Every check, in the order a person should read them. Fatal ones block starting."""
    checks = [
        check_host_binary(host_path),
        check_worker(worker_path, python_exe=python_exe),
        check_signing_library(python_exe=python_exe),
        check_gpu(),
    ]
    return checks, [c for c in checks if c.fatal and not c.ok]


def worker_command(python_exe, worker_path, job="run", extra=()):
    """The exact argv for one worker. `job` is one of the worker's own subcommands."""
    return [python_exe or sys.executable, str(worker_path), job, *extra]


def stop_command(pid):
    """How to stop a worker and its children WITHOUT os.killpg, which Windows lacks.

    ⚠ The worker spawns the prover as a child, and killing only the parent orphans a prove that goes
    on holding the GPU — `run-workers.sh` has that exact scar: "the ones already running were
    orphaned, not stopped". `taskkill /T` takes the tree.
    """
    if IS_WINDOWS:
        return ["taskkill", "/PID", str(pid), "/T", "/F"]
    return ["kill", "-TERM", str(pid)]


def summarise(checks):
    """A one-line verdict, for a status bar or a CLI run."""
    bad = [c for c in checks if not c.ok]
    fatal = [c for c in bad if c.fatal]
    if fatal:
        return f"cannot start: {fatal[0].name} — {fatal[0].detail}"
    if bad:
        return f"ready, with {len(bad)} warning(s): " + "; ".join(c.name for c in bad)
    return "ready"


def main(argv=None):
    """Headless preflight, so the checks can be run without a display or a GUI at all."""
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv or argv[0] in ("-h", "--help"):
        print("usage: python supervisor.py <host-binary> <hazync-worker> [identity-dir]")
        print("  Runs the same preflight the GUI runs, and prints it. No window needed.")
        return 0 if argv else 2
    host = argv[0]
    worker = argv[1] if len(argv) > 1 else ""
    ident = argv[2] if len(argv) > 2 else None
    checks, fatal = preflight(host, worker, identity_dir=ident)
    for c in checks:
        print(f"  {'ok  ' if c.ok else 'FAIL'} {c.name}: {c.detail}")
    print()
    print(summarise(checks))
    return 1 if fatal else 0


if __name__ == "__main__":
    sys.exit(main())
