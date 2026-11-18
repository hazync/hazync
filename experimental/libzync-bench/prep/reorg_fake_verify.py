#!/usr/bin/env python3
"""A stand-in for `host verify-any <rcpt> <lo> --check-end` for the end-to-end reorg test (reorg_e2e.py, REORG.md §12):
the coordinator runs with it as HAZYNC_HOST. The test's synthetic branch blocks carry no valid scripts or merkle roots
(prep checks neither; the guest does), so no guest can execute them, not even for a dev-mode receipt. What the board's
reorg handling depends on is the CHAIN a receipt was made on, so a receipt here is JSON naming it: {"lo", "hi", "in",
"out"}, the block hashes (state tip_hash bytes, hex) it starts after and ends on. Checked as lz_coord.rs verify_any
checks a real one, against prep's state files in $HAZYNC_LZ_STATE_DIR AS THEY ARE NOW: the state before lo (anchor),
and the board's state at hi (--check-end). The failure texts are lz_coord.rs's. Prints the RANGE-OK line.
"""
import hashlib, json, os, sys

a = sys.argv[1:]
if not a or a[0] != "verify-any" or "--check-end" not in a:
    print(f"reorg_fake_verify: only `verify-any <rcpt> <lo> --check-end`, not {a}", file=sys.stderr); sys.exit(2)
r = json.load(open(a[1]))
d = os.environ["HAZYNC_LZ_STATE_DIR"]
GEN = os.environ.get("GENESIS_TIP", "")

def tip(h):
    try:
        return bytes(json.load(open(os.path.join(d, f"state_{h}.json")))["tip_hash"]).hex()
    except (OSError, ValueError, KeyError, TypeError):
        return None

def link(x): return hashlib.sha256(bytes.fromhex(x)).hexdigest()

lo, hi = int(r["lo"]), int(r["hi"])
if lo != int(a[2]) and not (lo == 1 and int(a[2]) == 1):
    print(f"claimed start {a[2]} for a receipt from {lo}", file=sys.stderr); sys.exit(1)
if lo > 1:
    t = tip(lo - 1)
    if t is None:
        print(f"no leaf bundle (nor a state_{lo - 1}.json in HAZYNC_LZ_STATE_DIR): the state before block {lo} is unknown here",
              file=sys.stderr); sys.exit(1)
    if t != r["in"]:
        print(f"the receipt is not anchored on the state before block {lo} (anchor_id {link(r['in'])}, that state's digest "
              f"{link(t)})", file=sys.stderr); sys.exit(1)
t = tip(hi)
if t is None:
    print(f"the board has no state at block {hi} to check the receipt's end against", file=sys.stderr); sys.exit(1)
if t != r["out"]:
    print(f"the receipt ends on a state at block {hi} that is not the board's (its tip {r['out']}, the board's {t}): "
          f"not Bitcoin's chain", file=sys.stderr); sys.exit(1)
it = GEN if lo == 1 else link(r["in"])
print(f"RANGE-OK lo={lo} hi={hi} in_tip={it} out_tip={link(r['out'])} out_leaves=1 range_work=0 in_bhash={r['in']} "
      f"out_bhash={r['out']} image=fake lineage=fake anchored={int(lo == 1)} end=board fake=1")
