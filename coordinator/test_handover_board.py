#!/usr/bin/env python3
"""
The board side of a handover (docs/HANDOVER.md §5): LZ_HANDOVER_AT=H moves the board from image O to image N at block H.

  pre era (O's host):  no claim, pick or fold reaches H, and a submission ending at H or above is refused;
  post era (N's host): claims start at H+1 (block H is never claimable), O-era folds are not offered, a submission
                       starting at or below H is refused, and the spine that crosses H is recorded as vrange [1..H]
                       so the frontier walks from O's blocks through the handover into N's.

VERIFY_MODE=mock: the host's checks (RANGE-OK image/lineage, the guest's handover step) are tested by the dev-mode
rehearsal (experimental/libzync-bench/handover/board_rehearsal.sh), not here.

Usage:
  python3 test_handover_board.py            # assertions; exit 0 on success
  python3 test_handover_board.py --control  # no LZ_HANDOVER_AT: the board ignores the handover; these MUST fail
"""
import os
import sys
import tempfile

CONTROL = "--control" in sys.argv
H = 50

os.environ["COORD_DB"] = tempfile.NamedTemporaryFile(suffix=".db", delete=False).name
os.environ["COORD_SPINE"] = tempfile.mkdtemp(prefix="spine_")
os.environ["COORD_PROOFS"] = tempfile.mkdtemp(prefix="proofs_")
os.environ["VERIFY_MODE"] = "mock"
os.environ["COORD_ALLOW_MOCK"] = "1"
os.environ["TIP_CACHE_TTL"] = "0"
os.environ["LZ_HANDOVER_AT"] = "0" if CONTROL else str(H)
os.environ.setdefault("COORD_WEB", os.path.dirname(__file__))

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import server  # noqa: E402

server.init_db()
server.witness_available = lambda h: True
server.provable_tip = lambda: 1000
server.verify_sig = lambda pk, sig, msg: True
server.live_fold_claims = lambda now=None: set()

fails = []


def check(cond, what):
    print(f"  {'ok  ' if cond else 'FAIL'} {what}")
    if not cond:
        fails.append(what)


def tip(h):
    return f"{h:064x}"


def seed(blocks, image=None):
    """Per-block verified ranges, chained by tip from genesis (block 1's in_tip is GENESIS_TIP)."""
    c = server.db()
    for h in blocks:
        in_tip = server.GENESIS_TIP if h == 1 else tip(h - 1)
        c.execute("INSERT OR REPLACE INTO ranges(id,lo,hi,status,verified_at) VALUES(?,?,?,'verified',1)", (str(h), h, h))
        c.execute("INSERT OR REPLACE INTO vranges(id,lo,hi,in_tip,out_tip,pubkey,handle,ts,out_leaves,range_work,"
                  "in_bhash,out_bhash,image) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                  (str(h), h, h, in_tip, tip(h), "p" * 64, "t", 1.0, 0, "0", f"b{h-1}", f"b{h}", image))
    c.commit()
    c.close()
    server._frontier_invalidate()


def reset():
    c = server.db()
    c.execute("DELETE FROM vranges")
    c.execute("DELETE FROM ranges")
    c.commit()
    c.close()


def claim(n):
    return server.claim({"pubkey": f"{n:064x}", "handle": "tester", "nonce": f"n{n}"})


print("pre era (the predecessor's host, capped below H)")
os.environ["LZ_HANDOVER_ERA"] = "pre"
reset()
seed(range(1, H - 1))                      # 1..H-2 proven
code, got = claim(1)
check(code == 200 and got.get("range") == str(H - 1), f"the next claim is block {H - 1} (got {code} {got.get('range')})")
code, got = claim(2)
check(code == 409, f"nothing at or above H={H} is handed out (got {code} {got.get('range')})")
code, got = server.pick(None)
check(code == 404, f"pick suggests nothing at or above H (got {code} {got.get('range') if isinstance(got, dict) else got})")
check(server.era_refusal(H, H) is not None, f"a submission of block {H} is refused")
check(server.era_refusal(H - 1, H - 1) is None, f"a submission of block {H - 1} is not")
reset()
seed([H - 3, H - 2, H - 1])
c = server.db()
c.execute("INSERT OR REPLACE INTO vranges(id,lo,hi,in_tip,out_tip,pubkey,handle,ts,out_leaves,range_work,in_bhash,out_bhash) "
          "VALUES('50',50,50,'x','y','p','t',1,0,'0','a','b')")
c.commit(); c.close()
pairs = server.foldable(32)
check(not any(p["hi"] >= H for p in pairs), f"no fold reaching H is offered in the pre era ({pairs})")

print("post era (the new image's host, which pins the handover at H)")
os.environ["LZ_HANDOVER_ERA"] = "post"
reset()
seed(range(1, H))                          # O's 1..H-1
code, got = server.pick(None)
check(code == 200 and got.get("range") == str(H + 1), f"pick suggests {H + 1} (got {got.get('range') if isinstance(got, dict) else got})")
code, got = claim(3)
check(code == 200 and got.get("range") == str(H + 1), f"the first claim is block {H + 1}, never H (got {code} {got.get('range')})")
check(server.era_refusal(H, H) is not None, f"a submission of block {H} (not via the spine) is refused")
check(server.era_refusal(H - 1, H - 1) is not None, f"a new submission below H is refused")
check(server.era_refusal(H + 1, H + 1) is None, f"block {H + 1} is accepted")
seed([H - 3, H - 2])
pairs = server.foldable(32)
check(not any(p["lo"] <= H for p in pairs), f"no fold of the predecessor's ranges is offered ({pairs})")

# the handover spine: [1..H], image N, ending on the board's state at H
check(server.frontier_hi() == H - 1, f"before the handover the frontier is at {H - 1} (got {server.frontier_hi()})")
server.verify_spine = lambda receipt: (True, "ok", {"lo": 1, "hi": H, "in_tip": server.GENESIS_TIP, "out_tip": tip(H),
                                                    "out_leaves": 0, "range_work": "0", "in_bhash": "b0", "out_bhash": f"b{H}",
                                                    "image": "aa" * 32, "lineage": "bb" * 32})
os.makedirs(os.environ["COORD_SPINE"], exist_ok=True)
import json  # noqa: E402
with open(os.path.join(os.environ["COORD_SPINE"], "spine.json"), "w") as f:
    json.dump({"lo": 1, "hi": H - 1}, f)
code, got = server.submit_spine({"pubkey": "e" * 64, "sig": "f" * 128, "receipt": "aGFuZG92ZXI=", "handle": "op"})
check(code == 200, f"the handover spine [1..{H}] is accepted (got {code} {got})")
c = server.db()
row = c.execute("SELECT lo, hi, in_tip, out_tip, image FROM vranges WHERE id=?", (f"1-{H}",)).fetchone()
c.close()
check(row is not None and row["lo"] == 1 and row["hi"] == H and row["in_tip"] == server.GENESIS_TIP
      and row["image"] == "aa" * 32, f"it is recorded as vrange [1..{H}] with image N ({dict(row) if row else None})")
seed([H + 1, H + 2], image="aa" * 32)
server._frontier_invalidate()
fr = server._frontier_chain()[0]
check(fr == H + 2, f"the frontier walks through the handover into N's blocks: {fr} (want {H + 2})")
meta = server.handover_meta()
check(bool(meta) and meta["height"] == H and meta["era"] == "post" and meta["done"], f"/api/meta handover: {meta}")

if fails:
    print(f"FAIL: {len(fails)} check(s)")
    sys.exit(1)
print("OK: the board keeps each image to its blocks and crosses the handover once")
