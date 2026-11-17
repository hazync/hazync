#!/usr/bin/env python3
"""A worker stopped while it holds a claim hands the block back (#725).

The real HTTP handler on a loopback port and the real `hazync run` / `hazync release` against it. What is
checked:
- SIGTERM in the middle of a prove (run-workers.sh --stop, systemd, the app on Linux): the worker releases its
  claimed block, exits 128+15, and another key is offered that block straight away;
- the prover child is killed with it, not left running in its own session holding the GPU;
- a second SIGTERM during the hand-back, a SIGTERM while the claim request is still in flight, and Ctrl-C all
  still end with the block handed back;
- a prove that FAILS does not release: its claim lapses as before, so one bad block is not handed straight on;
- `hazync release <n> <claim>` (what the Windows app runs after it ends a worker with taskkill, which no handler
  sees) releases a block this key holds under that claim, says so plainly when there is nothing to release, and
  leaves a LATER claim of the same block by the same key alone;
- `hazync run` prints the claim's id on its "claimed block" line, which is where the app reads it from.

  python3 test_worker_release.py            # must PASS
  python3 test_worker_release.py --control  # the worker never releases; MUST FAIL
"""
import importlib.machinery
import importlib.util
import os
import signal
import stat
import sys
import tempfile
import threading
import time

_tmpdb = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
os.environ["COORD_DB"] = _tmpdb.name
os.environ["COORD_SPINE"] = tempfile.mkdtemp(prefix="spine_")
os.environ["COORD_PROOFS"] = tempfile.mkdtemp(prefix="proofs_")
os.environ["VERIFY_MODE"] = "mock"
os.environ["COORD_ALLOW_MOCK"] = "1"
os.environ["TIP_CACHE_TTL"] = "0"
os.environ["HAZYNC_HOME"] = tempfile.mkdtemp(prefix="hz_worker_")
os.environ.setdefault("COORD_WEB", os.path.dirname(os.path.abspath(__file__)))
CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))

sys.path.insert(0, HERE)
import server  # noqa: E402
server.init_db()
server.provable_tip = lambda: 1_000
server.witness_available = lambda h: True

httpd = server.ThreadingHTTPServer(("127.0.0.1", 0), server.H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
os.environ["COORD_URL"] = f"http://127.0.0.1:{httpd.server_address[1]}"

loader = importlib.machinery.SourceFileLoader("hazync_cli", os.path.join(HERE, "hazync"))
spec = importlib.util.spec_from_loader("hazync_cli", loader)
hz = importlib.util.module_from_spec(spec)
loader.exec_module(hz)

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey  # noqa: E402
from cryptography.hazmat.primitives import serialization  # noqa: E402


def key():
    sk = Ed25519PrivateKey.generate()
    return sk, sk.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()


SK, PK = key()
OTHER_SK, OTHER = key()
hz.identity = lambda: (SK, PK, "tester")
hz.cmd_submit = lambda args: None
if CONTROL:
    hz.release_claim = lambda rng, claim, sk, pub, post_fn=None: (False, "control: not released")
    print("CONTROL: the worker never releases -- the checks below MUST fail")

fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


def row(rng):
    c = server.db()
    r = c.execute("SELECT status, assignee FROM ranges WHERE id=?", (str(rng),)).fetchone()
    c.close()
    return (r["status"], r["assignee"]) if r else None


def reset():
    c = server.db()
    c.execute("DELETE FROM ranges")
    c.commit()
    c.close()
    server._release_seen.clear()


def other_claim():
    nonce, ts = os.urandom(8).hex(), int(time.time())
    return server.claim({"pubkey": OTHER, "handle": "other", "nonce": nonce, "ts": ts,
                         "sig": OTHER_SK.sign(f"claim:{nonce}:{ts}".encode()).hex()})[1].get("range")


# A stand-in prover: sleeps, so it is still running when the stop arrives. Its pid is written to a file.
work = tempfile.mkdtemp(prefix="hz_prover_")
pidfile = os.path.join(work, "prover.pid")
fake = os.path.join(work, "fake-host")
with open(fake, "w") as fh:
    fh.write(f"#!/bin/sh\necho $$ > {pidfile}\nexec sleep 60\n")
os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)
hz.HOSTBIN = fake


def alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    try:                                  # a zombie is gone for our purposes
        with open(f"/proc/{pid}/stat") as fh:
            return fh.read().split()[2] != "Z"
    except OSError:
        return False


print("== SIGTERM mid-prove: the claim is handed back ==")
reset()
seen = {}


def prove_then_get_stopped(args, claimed=False, **kw):
    seen["rng"], seen["claimed"] = args[0], claimed
    seen["held_before"] = row(args[0])
    threading.Timer(1.0, lambda: os.kill(os.getpid(), signal.SIGTERM)).start()
    # The real prove loop, on a prover that would run for a minute
    hz._prove_watched(["prove-range-bridge", args[0]], dict(os.environ), work, f"block {args[0]}")


hz.cmd_prove = prove_then_get_stopped
rc = None
try:
    hz.cmd_run([])
except SystemExit as e:
    rc = e.code
b = seen.get("rng")
check(seen.get("held_before") == ("claimed", PK), f"the worker had claimed block {b} ({seen.get('held_before')})")
check(rc == 128 + signal.SIGTERM, f"it exits 128+SIGTERM ({rc})")
check(b is not None and row(b) == ("open", PK), f"and block {b} is released, not held ({row(b) if b else None})")
check(other_claim() == b, "another key is offered that block straight away")
pid = int(open(pidfile).read().strip()) if os.path.exists(pidfile) else None
time.sleep(0.3)
check(pid is not None and not alive(pid), f"the prover child (pid {pid}) was killed, not left holding the GPU")
check(signal.getsignal(signal.SIGTERM) is not hz._raise_stop and signal.getsignal(signal.SIGTERM) != signal.SIG_IGN,
      f"the worker's SIGTERM handler is removed once run is over ({signal.getsignal(signal.SIGTERM)})")

print("== a second SIGTERM while handing back does not stop the release ==")
reset()
_quick = hz._post_quick


def quick_but_signalled(path, body):
    os.kill(os.getpid(), signal.SIGTERM)       # systemd, or --stop run twice, while the release is in flight
    time.sleep(0.2)
    return _quick(path, body)


hz._post_quick = quick_but_signalled
hz.cmd_prove = prove_then_get_stopped
rc = None
try:
    hz.cmd_run([])
except SystemExit as e:
    rc = e.code
except BaseException as e:                      # noqa: BLE001 - the failure this checks for
    rc = f"escaped: {type(e).__name__}"
hz._post_quick = _quick
b = seen.get("rng")
check(rc == 128 + signal.SIGTERM and row(b) == ("open", PK),
      f"the second signal is ignored and block {b} is still released (rc {rc}, {row(b)})")

print("== SIGTERM while the claim request is in flight ==")
reset()
_post = hz.post


def claim_but_signalled(path, body):
    if path == "/api/claim":
        os.kill(os.getpid(), signal.SIGTERM)   # lands before the answer: the worker does not know its block yet
        time.sleep(0.2)
    return _post(path, body)


hz.post = claim_but_signalled
proved = []
hz.cmd_prove = lambda args, claimed=False, **kw: proved.append(args[0])
rc = None
try:
    hz.cmd_run([])
except SystemExit as e:
    rc = e.code
except BaseException as e:                      # noqa: BLE001
    rc = f"escaped: {type(e).__name__}"
hz.post = _post
c = server.db()
held = [(r["id"], r["status"]) for r in c.execute("SELECT id, status FROM ranges WHERE assignee=?", (PK,))]
c.close()
check(rc == 128 + signal.SIGTERM and not proved and held and all(st == "open" for _, st in held),
      f"the claim completes, nothing is proved, and the block it was given is handed back (rc {rc}, {held})")

print("== Ctrl-C mid-prove ==")
reset()
hz.cmd_prove = lambda args, claimed=False, **kw: (seen.update(rng=args[0]),
                                                   os.kill(os.getpid(), signal.SIGINT), time.sleep(2))
rc = None
try:
    hz.cmd_run([])
except SystemExit as e:
    rc = e.code
except BaseException as e:                      # noqa: BLE001
    rc = f"escaped: {type(e).__name__}"
check(rc == 128 + signal.SIGINT and row(seen["rng"]) == ("open", PK), f"exits 130 and hands the block back (rc {rc})")
check(signal.getsignal(signal.SIGINT) is signal.default_int_handler, "and Ctrl-C behaves normally again afterwards")

print("== a prove that fails keeps its claim ==")
reset()


def prove_fails(args, claimed=False, **kw):
    seen["rng"] = args[0]
    sys.exit("block: the prover failed")


hz.cmd_prove = prove_fails
try:
    hz.cmd_run([])
except SystemExit:
    pass
check(row(seen["rng"]) == ("claimed", PK), f"a failed prove does not release block {seen['rng']} ({row(seen['rng'])})")

print("== hazync release <n> <claim> ==")
reset()


def my_claim():
    nonce, ts = os.urandom(16).hex(), int(time.time())
    return server.claim({"pubkey": PK, "handle": "tester", "nonce": nonce, "ts": ts,
                         "sig": SK.sign(f"claim:{nonce}:{ts}".encode()).hex()})[1]["range"], nonce


b, first = my_claim()
out = []
_print = print


def capture(*a, **k):
    out.append(" ".join(str(x) for x in a))


def release_cli(args):
    try:
        hz.cmd_release(args)
        return 0
    except SystemExit as e:
        return e.code


hz.print = capture
rc = release_cli([b, first])
check(rc == 0 and row(b) == ("open", PK), f"releases a block this key holds (rc {rc}, {row(b)}, {out[-1:]})")
rc = release_cli([b, first])
check(rc == 1 and "not released" in (out[-1] if out else ""),
      f"says so when there is nothing left to release (rc {rc}: {out[-1:] })")
b2, second = my_claim()
rc = release_cli([b, first])
check(b2 == b and rc == 1 and row(b) == ("claimed", PK) and "not this claim" in (out[-1] if out else ""),
      f"a late release of the first claim leaves the same key's re-claim of {b} alone (rc {rc}, {row(b)}, {out[-1:]})")
rc = release_cli([b])
check(rc not in (0, None) and row(b) == ("claimed", PK), f"a release that names no claim is a usage error (rc {str(rc)[:60]})")
hz.print = _print

print("== the claim's id is on the 'claimed block' line ==")
reset()
lines = []
hz.print = lambda *a, **k: lines.append(" ".join(str(x) for x in a))
hz.cmd_prove = lambda args, claimed=False, **kw: seen.update(rng=args[0])
try:
    hz.cmd_run([])
except SystemExit:
    pass
hz.print = _print
c = server.db()
nonce_row = c.execute("SELECT claim_nonce FROM ranges WHERE id=?", (str(seen["rng"]),)).fetchone()
c.close()
said = next((l for l in lines if l.startswith("claimed block")), "")
check(nonce_row is not None and f"claimed block {seen['rng']} (claim {nonce_row['claim_nonce']};" in said,
      f"`hazync run` names its claim ({said[:90]})")

httpd.shutdown()
print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
