#!/usr/bin/env python3
"""A libzync board accepts a receipt only if the host checked its END state against the board's own (--check-end).

Without that check a prover could take the board's real state before block N, mine its own block N on it (difficulty 1
early in the chain: seconds of work), prove it under the real image, and have a block that is not Bitcoin's accepted as
N; and a longer fork from genesis could replace the spine. The host refuses such a receipt under --check-end and marks
the RANGE-OK line `end=board`; the board passes the flag and refuses a line without the mark (an older host that
ignores the flag must fail closed, not be trusted).

A fake host records its arguments and prints a RANGE-OK line with or without `end=board`.

  python3 test_check_end.py            # must PASS
  python3 test_check_end.py --control  # server.py with the end check taken out; MUST FAIL
"""
import os, stat, sys, tempfile

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="checkend_")
HOST, ARGS = os.path.join(T, "host"), os.path.join(T, "args")
os.environ.update(HAZYNC_LIBZYNC="1", VERIFY_MODE="real", HAZYNC_HOST=HOST, HAZYNC_BRIDGE_OUT=os.path.join(T, "leaves"),
                  WITNESS_DIR=os.path.join(T, "w"), COORD_DB=os.path.join(T, "c.db"), COORD_STATE=os.path.join(T, "st"),
                  COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"))
sys.path.insert(0, HERE)
if CONTROL:
    # the board as it was: the same server.py with its end=board refusals and --check-end taken out
    import types
    src = open(os.path.join(HERE, "server.py")).read()
    for a, b in (('kv.get("end") != "board"', 'False'), ('"--check-end"', '"1"')):
        assert a in src, a
        src = src.replace(a, b)
    server = types.ModuleType("server"); server.__file__ = os.path.join(HERE, "server.py")
    sys.modules["server"] = server
    exec(compile(src, server.__file__, "exec"), server.__dict__)
    print("CONTROL: the board without the end check -- the checks below MUST fail")
else:
    import server  # noqa: E402

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

def fake_host(lo, hi, end_mark):
    tip = server.GENESIS_TIP if lo == 1 else "ab" * 32
    with open(HOST, "w") as f:
        f.write("#!/bin/sh\necho \"$@\" >> %s\n" % ARGS
                + f"echo 'RANGE-OK lo={lo} hi={hi} in_tip={tip} out_tip={'cd' * 32} out_leaves=3 range_work=0 "
                + f"in_bhash={'00' * 32} out_bhash={'11' * 32} anchored={int(lo == 1)}{' end=board' if end_mark else ''}'\n")
    os.chmod(HOST, os.stat(HOST).st_mode | stat.S_IEXEC)
    open(ARGS, "w").close()

rng = {"id": "5", "lo": 5, "hi": 5}
fake_host(5, 5, True)
ok, note, meta = server.verify_receipt(b"receipt", rng)
argv = open(ARGS).read().split()
check(ok and "--check-end" in argv and "5" in argv, f"a leaf whose end the host checked is accepted, and the board asked "
                                                   f"for the check ({' '.join(argv[1:])})")
fake_host(5, 5, False)
ok, note, meta = server.verify_receipt(b"receipt", rng)
check(not ok and "end state" in note, f"a leaf line without end=board is refused ({note[:80]})")

fake_host(1, 40, True)
ok, note, meta = server.verify_spine(b"spine")
argv = open(ARGS).read().splitlines()
check(ok and len(argv) == 1 and "--check-end" in argv[0].split(),
      f"a spine is verified ONCE, with --check-end ({len(argv)} host run(s))")
fake_host(1, 40, False)
ok, note, meta = server.verify_spine(b"spine")
check(not ok and "end state" in note, f"a spine line without end=board is refused ({note[:80]})")

if CONTROL:
    if fails == 4:
        print(f"CONTROL OK: all {fails} checks fail without the end check, as they must")
        sys.exit(0)
    print(f"CONTROL BROKEN: only {fails} of 4 checks failed without the end check -- this test proves nothing")
    sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
