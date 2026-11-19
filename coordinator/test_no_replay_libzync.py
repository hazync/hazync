#!/usr/bin/env python3
"""A libzync board has no replay path: the worker must say so, not fetch witnesses from block 1.

WHY THIS EXISTS. 11-18, a laptop on v0.26.1: every claim failed with "could not fetch witness for block 2 from
https://api.hazync.org: HTTP Error 404". When fetch_bundles() could not get a block's input file, cmd_prove fell back
to the legacy replay path (fetch_witnesses 1..hi), which a libzync board does not serve, so the error named block 2
of a range nobody asked for and hid the real cause. Now, when /api/meta says libzync, the worker exits with what is
wrong (the board did not serve the input file, or this worker cannot read it: update Hazync) and never replays.

No network: the worker's `get` is replaced with a stand-in board.

  python3 test_no_replay_libzync.py            # assertions; exit 0 on success
  python3 test_no_replay_libzync.py --control  # the guard is removed; MUST fail
"""
import importlib.machinery
import importlib.util
import json
import os
import sys
import tempfile
import urllib.error

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
os.environ["HAZYNC_HOME"] = tempfile.mkdtemp(prefix="hz_home_")
os.environ["BUNDLE_DIR"] = tempfile.mkdtemp(prefix="hz_bundles_")
os.environ["WITNESS_DIR"] = tempfile.mkdtemp(prefix="hz_wit_")

loader = importlib.machinery.SourceFileLoader("hazync_cli", os.path.join(HERE, "hazync"))
spec = importlib.util.spec_from_loader("hazync_cli", loader)
w = importlib.util.module_from_spec(spec)
loader.exec_module(w)
if CONTROL:
    w.no_replay_on_libzync = lambda lo, hi: None      # the worker as it was: fall back to the replay path

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


def board(libzync, witness):
    """A stand-in `get`: /api/meta says `libzync`; /api/witness/<h> answers witness(h) (bytes, or raises)."""
    asked = []

    def get(path):
        asked.append(path)
        if path == "/api/meta":
            return json.dumps({"libzync": libzync}).encode()
        if path.startswith("/api/witness/"):
            return witness(int(path.rsplit("/", 1)[1]))
        raise AssertionError(path)
    return get, asked


def http404(h):
    raise urllib.error.HTTPError(f"https://board/api/witness/{h}", 404, "Not Found", {}, None)


def attempt(lo, hi):
    """What cmd_prove does when fetch_bundles fails: the exit message, or None when it went on to replay."""
    w.WITNESS, w.BUNDLES = tempfile.mkdtemp(prefix="hz_wit_"), tempfile.mkdtemp(prefix="hz_bundles_")
    if w.fetch_bundles(lo, hi):
        return "bundles ok", []
    try:
        w.no_replay_on_libzync(lo, hi)
        w.fetch_witnesses(lo, hi)
    except SystemExit as e:
        return str(e.code), None
    return None, None


print("== the guard is wired into the prove path ==")
src = open(os.path.join(HERE, "hazync"), encoding="utf-8").read()
at_fallback = src.index("        fetch_witnesses(start, hi)")
check("no_replay_on_libzync(start, hi)" in src[at_fallback - 200:at_fallback],
      "cmd_prove calls no_replay_on_libzync right before the replay fallback")

print("== a libzync board that 404s block 7's input file ==")
w.get, asked = board(True, http404)
msg, _ = attempt(7, 7)
check(msg is not None and "replay from block 1" in msg and "block 7" in msg,
      f"the worker stops and says why, naming the block: {msg!r}")
check(not any(p == "/api/witness/1" or p == "/api/witness/2" for p in asked),
      f"no witness below the range was fetched (asked: {asked})")

print("== a libzync board serving a file this worker cannot read ==")
w.get, asked = board(True, lambda h: json.dumps({"block": h}).encode())
msg, _ = attempt(9, 9)
check(msg is not None and "cannot read the board's input files" in msg and "update Hazync" in msg,
      f"it says update Hazync: {msg!r}")

print("== a libzync board serving bytes that are not JSON and not LZLEAF ==")
w.get, asked = board(True, lambda h: b"\x00\x01binary")
msg, _ = attempt(10, 10)
check(msg is not None and "cannot read the board's input files" in msg, f"it says update Hazync: {msg!r}")

print("== an LZLEAF file is still taken (the normal path) ==")
w.get, asked = board(True, lambda h: b"LZLEAF" + bytes(10))
msg, _ = attempt(11, 11)
check(msg == "bundles ok", f"leaf bundle used: {msg!r}")

print("== a board that is not libzync keeps the replay fallback ==")
w.get, asked = board(False, lambda h: json.dumps({"block": h}).encode())
msg, _ = attempt(3, 3)
check(msg is None and "/api/witness/1" in asked, f"replayed from block 1 as before (asked: {asked})")

print()
if CONTROL:
    print("CONTROL: the guard removed ->", "FAILED as it must" if fails else "PASSED (the test is inert!)")
    sys.exit(0 if fails else 1)
print("PASS" if not fails else f"FAIL ({len(fails)})")
sys.exit(1 if fails else 0)
