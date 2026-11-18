#!/usr/bin/env python3
"""The offsite mirror under reorgs (docs/libzync/REORG.md §9, P5): a depth delay, and markers instead of overwrites.

  1. `copy --depth 6` uploads only proofs whose hi <= tip - 6; `check --depth 6` does not count the others missing;
  2. with no fresh tip file, `copy --depth` and `check --depth` upload nothing and exit 1 (never "deep enough");
  3. a reorg deeper than the delay: the coordinator's proofs/orphaned/1/ goes up under orphaned/1/, the marker
     reorgs/1.json covers the live names an orphaned proof holds in the store, and the new chain's proof of such a
     name goes to r1/<name> (never over the live key); `check` is clean; a second run changes nothing;
  4. a second reorg covering a name already under r1/: the newest proof goes to r2/;
  5. `spine --depth 6` copies the newest KEPT head that deep, not the live one; a kept head whose name the store holds
     with other bytes, explained by a local spine/orphaned/<E>/ head, goes to r<E>/.

  python3 test_offsite_reorg.py               # must PASS
  python3 test_offsite_reorg.py --control     # the depth filter taken out; MUST FAIL
"""
import hashlib, importlib.util, io, json, os, sys, tempfile, time
from contextlib import redirect_stdout

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("offsite", os.path.join(HERE, "deploy", "hazync-offsite-proofs.py"))
off = importlib.util.module_from_spec(spec)
spec.loader.exec_module(off)
if CONTROL:
    off.proof_hi = lambda name: 0                     # every proof reads as deep enough
    print("CONTROL: the depth filter taken out -- the checks below MUST fail")

MID = "46666b6f" + "0" * 56
P = "proofs-46666b6f/"
SP = "spine-46666b6f/"


class FakeS3:
    def __init__(self):
        self.objects = {}
    def get_paginator(self, name):
        s3 = self
        class Pg:
            def paginate(self, Bucket, Prefix):
                keys = sorted(k for (b, k) in s3.objects if b == Bucket and k.startswith(Prefix))
                yield {"Contents": [{"Key": k, "Size": len(s3.objects[(Bucket, k)])} for k in keys]}
        return Pg()
    def put_object(self, Bucket, Key, Body, ContentType=None):
        assert (Bucket, Key) not in self.objects, f"overwrite of {Key}"
        self.objects[(Bucket, Key)] = Body.read()
    def upload_fileobj(self, Fileobj, Bucket, Key, ExtraArgs=None, Callback=None, Config=None):
        assert (Bucket, Key) not in self.objects, f"overwrite of {Key}"
        self.objects[(Bucket, Key)] = Fileobj.read()
    def get_object(self, Bucket, Key):
        return {"Body": io.BytesIO(self.objects[(Bucket, Key)])}
    def keys(self):
        return sorted(k for _, k in self.objects)


fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

T = tempfile.mkdtemp(prefix="offreorg_")
PROOFS, SPINE, TIP = os.path.join(T, "proofs"), os.path.join(T, "spine"), os.path.join(T, "node_tip")
KEYS = os.path.join(T, "keys")
open(KEYS, "w").write("kid secret https://x.r2.cloudflarestorage.com b\n")
os.makedirs(PROOFS); os.makedirs(os.path.join(SPINE, "heads"))
old = time.time() - 7200

def proof(name, body, d=PROOFS):
    os.makedirs(d, exist_ok=True)
    p = os.path.join(d, name); open(p, "wb").write(body); os.utime(p, (old, old))

def tip(h, age=0):
    open(TIP, "w").write(f"{h}\n"); t = time.time() - age; os.utime(TIP, (t, t))

def run(s3, mode, *extra):
    buf = io.StringIO()
    with redirect_stdout(buf):
        rc = off.main([mode, "--keys", KEYS, "--bucket", "b", "--proofs", PROOFS, "--spine", SPINE, "--method-id", MID,
                       "--bwlimit-mbit", "0", "--threads", "2", "--depth", "6", "--tip-file", TIP, *extra], client=s3)
    return rc, buf.getvalue()

s3 = FakeS3()
for h in range(1, 21):
    proof(f"proof_{h}.bin", b"A%d" % h)
proof("proof_17-20.bin", b"A17-20")
tip(20)
rc, log = run(s3, "copy")
up = [k for k in s3.keys() if k.startswith(P)]
check(rc == 0 and f"{P}proof_14.bin" in up and f"{P}proof_15.bin" not in up and f"{P}proof_17-20.bin" not in up,
      f"1: tip 20, depth 6: proofs up to 14 uploaded, 15..20 and [17..20] wait ({len(up)} uploaded)")
rc, log = run(s3, "check")
check(rc == 0, f"1: check does not count the shallow ones missing (exit {rc})")
tip(20, age=7200)
n0 = len(s3.keys())
rc1, _ = run(s3, "copy"); rc2, _ = run(s3, "check")
check(rc1 == 1 and rc2 == 1 and len(s3.keys()) == n0, f"2: a stale tip file: nothing uploaded, copy and check exit 1 ({rc1}, {rc2})")

# the board proves on to 40, the store gets up to 34; then a reorg at 17 (depth > 6): 18..34 orphaned
for h in range(21, 41):
    proof(f"proof_{h}.bin", b"A%d" % h)
tip(40); run(s3, "copy")
check(f"{P}proof_34.bin" in s3.keys() and f"{P}proof_35.bin" not in s3.keys(), "the store holds A's proofs up to 34")
od = os.path.join(PROOFS, "orphaned", "1")
for h in range(18, 30):                                      # the coordinator's rewind, half way through its moves
    os.makedirs(od, exist_ok=True); os.replace(os.path.join(PROOFS, f"proof_{h}.bin"), os.path.join(od, f"proof_{h}.bin"))
rc, log = run(s3, "copy")
check(rc == 0 and f"{P}reorgs/1.json" not in s3.keys() and "marker deferred" in log,
      "3: an epoch whose rewind is not complete (no DONE): no marker written yet (it would miss names)")
for h in range(30, 41):
    os.replace(os.path.join(PROOFS, f"proof_{h}.bin"), os.path.join(od, f"proof_{h}.bin"))
os.replace(os.path.join(PROOFS, "proof_17-20.bin"), os.path.join(od, "proof_17-20.bin"))
open(os.path.join(od, "DONE"), "w").write("{}")
for h in range(18, 45):
    proof(f"proof_{h}.bin", b"Bnew%d" % h)                     # chain B, other bytes, other sizes
tip(44)
rc, log = run(s3, "copy")
mk = json.loads(s3.objects[("b", f"{P}reorgs/1.json")]) if ("b", f"{P}reorgs/1.json") in s3.objects else {}
cov = sorted(int(n[6:-4]) for n in mk.get("covered", {}) if "-" not in n)
check(rc == 0 and cov == list(range(18, 35)) and "proof_17-20.bin" in mk.get("covered", {}),
      f"3: marker reorgs/1.json covers the live names A's orphaned proofs hold (18..34 and [17..20]): {cov[:2]}..{cov[-1:]}")
check(all(f"{P}orphaned/1/proof_{h}.bin" in s3.keys() for h in range(18, 41)), "3: the orphaned proofs copied under orphaned/1/")
check(s3.objects[("b", f"{P}proof_18.bin")] == b"A18" and s3.objects.get(("b", f"{P}r1/proof_18.bin")) == b"Bnew18"
      and s3.objects.get(("b", f"{P}proof_35.bin")) == b"Bnew35" and f"{P}proof_39.bin" not in s3.keys(),
      "3: B's 18 under r1/ (the live key keeps A's), B's 35 (never held by A) under its live name, 39+ wait")
rc, log = run(s3, "check")
n1 = len(s3.keys())
rc2, _ = run(s3, "copy")
check(rc == 0 and rc2 == 0 and len(s3.keys()) == n1, f"3: check clean ({rc}); a second copy changes nothing")

# a second reorg at 19, covering 20 (already under r1/) and 35 (live)
od2 = os.path.join(PROOFS, "orphaned", "2")
for h in range(20, 45):
    os.makedirs(od2, exist_ok=True); os.replace(os.path.join(PROOFS, f"proof_{h}.bin"), os.path.join(od2, f"proof_{h}.bin"))
open(os.path.join(od2, "DONE"), "w").write("{}")
for h in range(20, 46):
    proof(f"proof_{h}.bin", b"Cnewer%d" % h)
tip(45)
rc, log = run(s3, "copy")
check(rc == 0 and s3.objects.get(("b", f"{P}r2/proof_20.bin")) == b"Cnewer20" and s3.objects[("b", f"{P}r1/proof_20.bin")] == b"Bnew20"
      and s3.objects.get(("b", f"{P}r2/proof_35.bin")) == b"Cnewer35" and run(s3, "check")[0] == 0,
      "4: a second reorg: C's 20 under r2/ (r1/ keeps B's), C's 35 under r2/; check clean")

# a restore reads the markers: per name, the newest r<E>/ copy, else the live key
rc, log = run(s3, "restore-plan")
plan = dict(l.split() for l in log.splitlines() if l.startswith("proof_"))
check(rc == 0 and plan.get("proof_20.bin") == f"{P}r2/proof_20.bin" and plan.get("proof_18.bin") == f"{P}r1/proof_18.bin"
      and plan.get("proof_5.bin") == f"{P}proof_5.bin",
      "restore-plan: 20 from r2/ (newest), 18 from r1/, 5 from its live name")

# the spine: kept heads 30 and 40, live head 44; tip 45 -> the newest kept head <= 39 is 30
def head(d, hi, body):
    os.makedirs(d, exist_ok=True)
    meta = {"lo": 1, "hi": hi, "sha256": hashlib.sha256(body).hexdigest(), "bytes": len(body)}
    open(os.path.join(d, f"spine_{hi}.bin"), "wb").write(body); open(os.path.join(d, f"spine_{hi}.json"), "w").write(json.dumps(meta))
    return meta
head(os.path.join(SPINE, "heads"), 30, b"spineA30"); head(os.path.join(SPINE, "heads"), 40, b"spineA40")
m44 = head(os.path.join(SPINE, "heads"), 44, b"spineA44")
open(os.path.join(SPINE, "spine.bin"), "wb").write(b"spineA44"); open(os.path.join(SPINE, "spine.json"), "w").write(json.dumps(m44))
rc, log = run(s3, "spine")
sk = [k for k in s3.keys() if k.startswith(SP)]
check(rc == 0 and sk == [f"{SP}spine_1-30.bin", f"{SP}spine_1-30.json"], f"5: tip 45: the kept head 30 is copied, not 44 or 40 ({sk})")
tip(46); run(s3, "spine")                                   # the kept head 40 goes up
# a reorg at 35: head 40 orphaned (spine/orphaned/3/), a new head 40 of another chain kept
o3 = os.path.join(SPINE, "orphaned", "3"); os.makedirs(o3)
for x in ("bin", "json"):
    os.replace(os.path.join(SPINE, "heads", f"spine_40.{x}"), os.path.join(o3, f"spine_40.{x}"))
os.remove(os.path.join(SPINE, "heads", "spine_44.bin")); os.remove(os.path.join(SPINE, "heads", "spine_44.json"))
head(os.path.join(SPINE, "heads"), 40, b"spineC40-longer"); tip(50)
rc, log = run(s3, "spine")
check(rc == 0 and s3.objects[("b", f"{SP}spine_1-40.bin")] == b"spineA40" and s3.objects.get(("b", f"{SP}r3/spine_1-40.bin")) == b"spineC40-longer"
      and f"{SP}orphaned/3/spine_40.bin" in s3.keys(),
      "5: the new head 40 under r3/ (the live name keeps the orphaned one), the orphaned head copied under orphaned/3/")

if CONTROL:
    if fails:
        print(f"CONTROL OK: {fails} check(s) fail, as they must"); sys.exit(0)
    print("CONTROL BROKEN: nothing failed -- this test proves nothing"); sys.exit(1)
print("PASS" if fails == 0 else f"FAIL ({fails})")
sys.exit(1 if fails else 0)
