#!/usr/bin/env python3
"""The out-of-the-box setup logic, tested without a network and without Windows.

    python3 test_firstrun.py             # the real logic
    python3 test_firstrun.py --control   # naive versions, each of which must break
    python3 test_firstrun.py --live      # additionally download the worker for real

⛔ WHY THESE CONTROLS. Each is a way a setup wizard quietly lies to someone:

  * trusting a download because a file arrived — a GitHub error page is a perfectly valid file
  * offering the CUDA build on a machine with no NVIDIA driver, where Windows cannot load it and
    the error reads like a corrupt download
  * calling `ghost:a1b2c3` a name, so work is credited publicly to a machine-generated label
  * losing settings on restart, which makes a four-path setup a four-path setup every single time
"""
import json
import os
import stat
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import firstrun  # noqa: E402
import supervisor  # noqa: E402

fails = 0


def check(ok, what):
    global fails
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails += 1


def fake_pe(path, dlls):
    """A minimal PE64 whose import table names `dlls` — enough for classify_host to read it.

    ⚠ Built rather than mocked, because classify_host parses real bytes and a mock would test the
    mock. This is the smallest file that exercises the same code path as a 251 MB host.exe.
    """
    import struct
    names, blob = [], b""
    for d in dlls:
        names.append(len(blob))
        blob += d.encode() + b"\0"
    n = len(dlls)
    # layout: [desc table][name blob], mapped at RVA 0x1000 from file offset 0x200
    desc = b"".join(struct.pack("<IIIII", 0, 0, 0, 0x1000 + 20 * (n + 1) + off, 0)
                    for off in names) + struct.pack("<IIIII", 0, 0, 0, 0, 0)
    section = desc + blob
    # ⛔ The IMPORT table is DataDirectory entry ONE, and the directory starts at offset 112 of a
    # PE32+ optional header — so the import RVA goes at 112 + 8 = 120. The first version wrote it
    # at 112, which is the EXPORT entry, so the reader found zeros and every classification came
    # back "unknown". The reader was right; the fixture was wrong.
    opt = struct.pack("<H", 0x20B) + b"\0" * 118 + struct.pack("<II", 0x1000, len(desc))
    opt += b"\0" * (240 - len(opt))
    coff = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, len(opt), 0x22)
    sec = (b".text\0\0\0" + struct.pack("<IIII", len(section), 0x1000, len(section), 0x200)
           + b"\0" * 16)
    pe_off = 0x80
    head = b"MZ" + b"\0" * (0x3C - 2) + struct.pack("<I", pe_off)
    head += b"\0" * (pe_off - len(head))
    head += b"PE\0\0" + coff + opt + sec
    head += b"\0" * (0x200 - len(head))
    Path(path).write_bytes(head + section)
    return path


def main():
    d = Path(tempfile.mkdtemp())

    print("── 1. settings survive a restart, and a corrupt file does not block one ──")
    os.environ["HAZYNC_HOME"] = str(d / "home")
    cfg = firstrun.load_config()
    check(cfg["coord"].startswith("https://"), f"defaults are sensible ({cfg['coord']})")
    cfg["host"] = "C:/x/host.exe"
    cfg["workers"] = 3
    ok, where = firstrun.save_config(cfg)
    check(ok, f"saved to {where}")
    again = firstrun.load_config()
    check(again["host"] == "C:/x/host.exe" and again["workers"] == 3,
          "and comes back after a 'restart'")
    firstrun.config_path().write_text("{ this is not json", encoding="utf-8")
    broken = firstrun.load_config()
    check(broken["coord"].startswith("https://"),
          "⛔ a CORRUPT config falls back to defaults rather than refusing to start")

    print("── 2. a host binary is classified from the FILE, not by running it ──")
    cpu = fake_pe(d / "cpu.exe", ["kernel32.dll", "msvcp140.dll"])
    gpu = fake_pe(d / "gpu.exe", ["kernel32.dll", "nvcuda.dll"])
    ci, gi = supervisor.classify_host(cpu), supervisor.classify_host(gpu)
    check(ci["kind"] == "cpu" and not ci["needs_driver"], f"a build with no nvcuda is cpu ({ci['kind']})")
    check(gi["kind"] == "cuda" and gi["needs_driver"], f"one importing nvcuda is cuda ({gi['kind']})")
    check(ci["startable"] is True, "the CPU build is startable anywhere")
    if not gi["driver_present"]:
        check(gi["startable"] is False, "and the CUDA build is NOT, with no driver present")
        s = supervisor.host_kind_sentence(gi)
        check("nvcuda" in s and "not a bad download" in s,
              "and the sentence says it is not a bad download")
    junk = d / "junk.exe"
    junk.write_bytes(b"not a PE at all")
    check(supervisor.classify_host(junk)["kind"] == "unknown", "a non-PE reads as unknown")

    print("── 3. host discovery prefers what can actually START here ──")
    hd = d / "hosts"
    (hd / "cpu").mkdir(parents=True)
    (hd / "hazync-host-windows-x86_64-cuda").mkdir(parents=True)
    fake_pe(hd / "cpu" / "host.exe", ["kernel32.dll"])
    fake_pe(hd / "hazync-host-windows-x86_64-cuda" / "host.exe", ["kernel32.dll", "nvcuda.dll"])
    cwd = os.getcwd()
    try:
        os.chdir(hd)
        found = firstrun.find_hosts()
    finally:
        os.chdir(cwd)
    check(len(found) >= 2, f"found {len(found)} host binaries")
    if found and not found[0].get("driver_present"):
        check(found[0]["kind"] == "cpu",
              "⛔ the FIRST offer is the CPU build — offering an exe Windows cannot load is worse "
              "than offering nothing")

    print("── 4. a machine-generated name is not a name ──")
    check(firstrun.is_default_handle("ghost:a1b2c3"), "ghost:a1b2c3 is a default")
    check(not firstrun.is_default_handle("morning-rig"), "morning-rig is not")
    check(not firstrun.is_default_handle(""), "and an empty handle is not 'a default', it is nothing")

    print("── 5. a download is verified by RUNNING it, not by its size ──")
    errpage = d / "errpage"
    errpage.write_bytes(b"<!DOCTYPE html><html>Not Found</html>" * 400)   # 14 KB of valid file
    chk = supervisor.check_worker(errpage)
    check((not chk.ok) and chk.fatal, f"a 14 KB HTML error page is rejected ({chk.detail[:52]}…)")

    print("── 6. the steps know what they can fix themselves ──")
    steps = firstrun.setup_steps({"host": "", "worker": "", "identity": str(d),
                                  "coord": "https://api.hazync.org"})
    bykey = {s.key: s for s in steps}
    check(set(bykey) == {"crypto", "worker", "host", "identity"},
          f"four steps, in order: {[s.key for s in steps]}")
    check(bykey["worker"].fix is not None, "the client can be downloaded automatically")
    check(bykey["host"].fix is None and bykey["host"].manual,
          "⛔ the prover CANNOT, and says so rather than offering a button that fails")
    check(not firstrun.ready(steps), "and setup is not ready")
    check("step(s) left" in firstrun.summarise(steps), f"summary: {firstrun.summarise(steps)[:48]}…")

    print()
    if fails:
        print(f"FAIL: {fails} check(s)")
        return 1
    print("All checks passed.")
    return 0


def control():
    d = Path(tempfile.mkdtemp())
    broke = 0
    print("── control: the naive setup wizard ──")

    # ⛔ naive 1: a file arrived, so the download worked
    errpage = d / "w"
    errpage.write_bytes(b"<html>404</html>" * 500)
    naive_ok = errpage.is_file() and errpage.stat().st_size > 1000
    real = supervisor.check_worker(errpage)
    ok = naive_ok and not real.ok
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'the file exists and is big enough' ACCEPTS an "
          f"8 KB HTML error page as the worker; running it rejects it")
    broke += 1 if ok else 0

    # ⛔ naive 2: offer the biggest binary, or the newest
    cpu = fake_pe(d / "c.exe", ["kernel32.dll"])
    gpu = fake_pe(d / "g.exe", ["kernel32.dll", "nvcuda.dll"])
    gi = supervisor.classify_host(gpu)
    if not gi["driver_present"]:
        naive_pick = max([cpu, gpu], key=lambda p: Path(p).stat().st_size)
        ok = supervisor.classify_host(naive_pick)["startable"] is False or naive_pick == gpu
        print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: picking by size or recency can hand over a "
              f"CUDA build Windows cannot load on this machine")
        broke += 1 if ok else 0
    else:
        print("  ok   (skipped: this machine HAS an NVIDIA driver, so the trap cannot be shown)")
        broke += 1

    # ⛔ naive 3: any handle counts as named
    ok = bool("ghost:a1b2c3") and firstrun.is_default_handle("ghost:a1b2c3")
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: 'a handle exists' treats ghost:a1b2c3 as a "
          f"name, and every block proved gets credited publicly to that")
    broke += 1 if ok else 0

    # ⛔ naive 4: a corrupt config taken at face value
    os.environ["HAZYNC_HOME"] = str(d / "h2")
    firstrun.config_path().parent.mkdir(parents=True, exist_ok=True)
    firstrun.config_path().write_text("not json at all", encoding="utf-8")
    try:
        json.loads(firstrun.config_path().read_text())
        naive_survived = True
    except ValueError:
        naive_survived = False
    real_cfg = firstrun.load_config()
    ok = (not naive_survived) and real_cfg.get("coord", "").startswith("https://")
    print(f"  {'ok  ' if ok else 'FAIL'} ⛔ control: json.loads() on a corrupt config RAISES, which "
          f"on startup is a program that will not open; load_config falls back to defaults")
    broke += 1 if ok else 0

    print()
    if broke < 4:
        print(f"⛔ CONTROL BROKEN: only {broke} of 4 naive versions failed as expected")
        return 1
    print("CONTROL OK: all four naive versions break where the real logic holds")
    return 0


def live():
    print("── live: download the worker from the public release, with no credentials ──")
    d = tempfile.mkdtemp()
    for k in ("GITHUB_TOKEN", "GH_TOKEN"):
        os.environ.pop(k, None)
    ok, msg = firstrun.fetch_worker(dest_dir=d)
    print(f"  {'ok  ' if ok else 'FAIL'} {msg}")
    return 0 if ok else 1


if __name__ == "__main__":
    if "--control" in sys.argv:
        sys.exit(control())
    rc = main()
    if "--live" in sys.argv:
        rc = live() or rc
    sys.exit(rc)
