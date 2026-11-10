#!/usr/bin/env python3
"""deploy/libzync/carry-contributors.py: identities carry over, CORE block counts do not, nothing else changes.

  1. a plan (no --apply) changes nothing in either database;
  2. --apply copies pubkey, handle, first_seen, last_version(_at) with blocks 0;
  3. a pubkey already on the libzync board is left exactly as it is (its handle and blocks);
  4. the original database is byte-for-byte unchanged;
  5. a missing table is exit 2.

  python3 test_libzync_carry.py            # must PASS
  python3 test_libzync_carry.py --control  # the copy carries CORE block counts; MUST FAIL
"""
import hashlib, importlib.util, io, os, sqlite3, sys, tempfile
from contextlib import redirect_stdout

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("carry", os.path.join(HERE, "deploy", "libzync", "carry-contributors.py"))
carry = importlib.util.module_from_spec(spec); spec.loader.exec_module(carry)
fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

SCHEMA = "CREATE TABLE contributors(pubkey TEXT PRIMARY KEY, handle TEXT, blocks INTEGER DEFAULT 0, first_seen REAL, last_version TEXT, last_version_at REAL)"
T = tempfile.mkdtemp(prefix="carry_")
OLD, NEW = os.path.join(T, "old.db"), os.path.join(T, "new.db")
o = sqlite3.connect(OLD); o.execute(SCHEMA)
o.executemany("INSERT INTO contributors VALUES(?,?,?,?,?,?)", [
    ("aa" * 32, "alice", 5000, 100.0, "v0.22.1", 200.0),
    ("bb" * 32, "bob", 12, 110.0, None, None),
    ("cc" * 32, "carol", 7, 120.0, "v0.21.6", 150.0)]); o.commit(); o.close()
n = sqlite3.connect(NEW); n.execute(SCHEMA)
n.execute("INSERT INTO contributors VALUES(?,?,?,?,?,?)", ("cc" * 32, "carol-lz", 4, 300.0, "v0.23.0", 310.0)); n.commit(); n.close()
digest = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
old_sha, new_sha = digest(OLD), digest(NEW)

if CONTROL:
    carry.COLS = carry.COLS + ("blocks",)
    print("CONTROL: the copy carries CORE block counts -- the checks below MUST fail")

buf = io.StringIO()
with redirect_stdout(buf):
    rc = carry.main(["--from", OLD, "--to", NEW])
check(rc == 0 and "2 to carry" in buf.getvalue() and digest(NEW) == new_sha, "a plan copies nothing and says 2 to carry")
with redirect_stdout(io.StringIO()):
    try:
        rc = carry.main(["--from", OLD, "--to", NEW, "--apply"])
    except Exception as e:
        rc = f"raised {e!r}"
rows = {r[0]: r[1:] for r in sqlite3.connect(NEW).execute("SELECT pubkey,handle,blocks,first_seen,last_version,last_version_at FROM contributors")}
check(rc == 0 and rows.get("aa" * 32) == ("alice", 0, 100.0, "v0.22.1", 200.0) and rows.get("bb" * 32) == ("bob", 0, 110.0, None, None),
      f"--apply carries identity and version with blocks 0 ({rows.get('aa' * 32)})")
check(rows.get("cc" * 32) == ("carol-lz", 4, 300.0, "v0.23.0", 310.0), "a pubkey already on the libzync board is left as it is")
check(digest(OLD) == old_sha, "the original database is unchanged")
E = os.path.join(T, "empty.db"); sqlite3.connect(E).close()
with redirect_stdout(io.StringIO()):
    rc = carry.main(["--from", E, "--to", NEW])
check(rc == 2, "a database with no contributors table is exit 2")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
