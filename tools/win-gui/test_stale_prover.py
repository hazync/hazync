#!/usr/bin/env python3
"""What the app says about a prover or worker that is an old copy, and the fix it offers (headless).

    python3 test_stale_prover.py             # the real logic
    python3 test_stale_prover.py --control   # the advice as it was: must fail

⛔ THE CASE (11-18, v0.26.1 on a real laptop). The startup check said "The prover's guest id does not match the
coordinator's ... Download the prover again." The prover path is REMEMBERED across upgrades and still pointed at an
old host.exe making the v1 image 37987b85, and an upgrade replaces only prover\\ and prover-cpu\\ beside the app, so
downloading changed nothing. Then, with the prover switched, every claim failed with a 404 at block 2: the remembered
WORKER was an old copy too, unable to read the board's LZLEAF input files.

Asserted: the advice names the remembered path and its image; it offers the installed prover when that one makes the
board's image (and not when it does not); it says "install the latest Hazync" when the remembered prover IS the
installed one; "Update Hazync to keep proving" when the board has moved past this app's image; an old worker is
caught from its file alone and the installed one offered; the generic explanation no longer says "download".
"""
import os as _os, sys as _sys
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import winconsole as _wc  # noqa: E402
_wc.fix()

import sys
import tempfile
from pathlib import Path

import supervisor  # noqa: E402

CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


OLD = "37987b85ec665970" + "0" * 48
BOARD = supervisor.CANONICAL_METHOD_ID
MOVED = "5" * 64


def advice(host, got, board, kind="cuda", installed=None, images=None):
    if CONTROL:     # the advice as it was: the generic sentence, nothing named, nothing offered
        return {"head": "Stopped", "text": supervisor.explain("method_id"), "action": None, "path": "", "kind": kind}
    return supervisor.stale_prover_advice(host, got, board, kind=kind, installed=installed or {},
                                          image_of=(images or {}).get)


def main():
    d = Path(tempfile.mkdtemp())
    old, inst, inst_cpu = d / "old" / "host.exe", d / "app" / "prover" / "host.exe", d / "app" / "prover-cpu" / "host.exe"
    for p in (old, inst, inst_cpu):
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(b"x")

    print("── the laptop: an old remembered prover, the installed one makes the board's image ──")
    a = advice(str(old), OLD, BOARD, installed={"cuda": str(inst), "cpu": str(inst_cpu)},
               images={str(inst): BOARD, str(inst_cpu): BOARD})
    check(a and str(old) in a["text"] and "37987b85" in a["text"] and BOARD[:8] in a["text"],
          f"names the remembered path, its image and the board's: {a and a['text']!r}")
    check(a and a["action"] == "switch" and a["path"] == str(inst) and a["kind"] == "cuda",
          f"offers the installed prover of the same kind, one click: {a and (a['action'], a['path'])}")
    check(a and "download" not in a["text"].lower(), "it does not say download (that changes nothing here)")

    print("── the installed CUDA prover is wrong too, the CPU one is right ──")
    a = advice(str(old), OLD, BOARD, installed={"cuda": str(inst), "cpu": str(inst_cpu)},
               images={str(inst): OLD, str(inst_cpu): BOARD})
    check(a and a["action"] == "switch" and a["path"] == str(inst_cpu) and a["kind"] == "cpu",
          f"offers the one that makes the board's image: {a and (a['action'], a['path'], a['kind'])}")

    print("── the remembered prover IS the installed one ──")
    a = advice(str(inst), OLD, BOARD, installed={"cuda": str(inst)}, images={str(inst): OLD})
    check(a and a["action"] == "update" and "install the latest Hazync" in a["text"],
          f"says install the latest Hazync: {a and a['text']!r}")

    print("── the board has moved past this app's image ──")
    a = advice(str(inst), BOARD, MOVED, installed={"cuda": str(inst)}, images={str(inst): BOARD})
    check(a and a["head"] == "Update Hazync to keep proving" and a["action"] == "update",
          f"'Update Hazync to keep proving', with Update: {a and (a['head'], a['action'])}")

    print("── from source: nothing installed ──")
    a = advice(str(old), OLD, BOARD)
    check(a and a["action"] is None and str(old) in a["text"], f"names it, offers no button: {a and a['action']}")
    check(CONTROL or supervisor.stale_prover_advice(str(inst), BOARD, BOARD, installed={}) is None,
          "a prover making the board's image needs no advice")

    print("── the generic explanation ──")
    e = supervisor.explain("METHOD_ID 37987b85...") or ""
    check("download the prover again" not in e.lower(), f"no longer 'Download the prover again': {e!r}")

    print("── the worker ──")
    oldw, instw = d / "old-worker", d / "app" / "hazync-worker"
    oldw.write_text('#!/usr/bin/env python3\nimport sys\nVERSION = "0.21.0"\n', encoding="utf-8")
    instw.write_text('#!/usr/bin/env python3\nimport sys\nVERSION = "0.26.1"\n# LZLEAF\n', encoding="utf-8")
    f = supervisor.worker_facts(oldw)
    check(f == {"version": "0.21.0", "lzleaf": False}, f"an old worker's file says what it is: {f}")
    check(supervisor.worker_facts(instw)["lzleaf"], "a current one reads LZLEAF")
    w = None if CONTROL else supervisor.stale_worker_advice(str(oldw), installed=str(instw))
    check(w and w["action"] == "switch" and w["worker"] == str(instw) and str(oldw) in w["text"]
          and "cannot read the board's input files" in w["text"],
          f"an old worker is named and the installed one offered: {w and w['text']!r}")
    check(supervisor.stale_worker_advice(str(instw), installed=str(instw)) is None, "the installed one is fine")
    r = supervisor.check_worker_current(str(oldw))
    check(not r.ok and r.fatal, f"Start refuses an old worker: {r.detail}")
    m = supervisor.merge_advice(
        supervisor.stale_prover_advice(str(old), OLD, BOARD, installed={"cuda": str(inst)}, image_of={str(inst): BOARD}.get),
        w)
    check(m and m["action"] == "switch" and m["path"] == str(inst) and m.get("worker") == str(instw),
          f"both old: one fix switches both: {m and (m['action'], m['path'], m.get('worker'))}")

    print()
    if CONTROL:
        print("CONTROL:", "FAILED as it must" if fails else "PASSED (the test is inert!)")
        return 0 if fails else 1
    print("PASS" if not fails else f"FAIL ({len(fails)})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
