#!/usr/bin/env python3
"""On a libzync coordinator (HAZYNC_LIBZYNC=1) every part of the board names a block's bundle the same way.

  1. the sponsor bot's has_bundle(h) agrees with the coordinator's own bridge file for h: a leaf_<h>.bin counts,
     a stray bundle_<h>.json does not (the coordinator would not serve it), a missing block does not;
  2. the worker's bundle_name() names a served LZLEAF bundle leaf_<h>.bin (what a libzync host reads) and a JSON
     bundle bundle_<h>.json, so `hazync selftest` pre-flights a libzync host on the file it can read.

  python3 test_libzync_coord_files.py            # must PASS
  python3 test_libzync_coord_files.py --control  # the bot looks for the original bridge's file; MUST FAIL
"""
import importlib.machinery, importlib.util, os, sys, tempfile

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
T = tempfile.mkdtemp(prefix="lzfiles_")
BR, WI = os.path.join(T, "leaves"), os.path.join(T, "witnesses")
os.makedirs(BR); os.makedirs(WI)
os.environ.update(HAZYNC_LIBZYNC="1", HAZYNC_BRIDGE_OUT=BR, WITNESS_DIR=WI, COORD_DB=os.path.join(T, "c.db"),
                  COORD_STATE=os.path.join(T, "st"), COORD_PROOFS=os.path.join(T, "p"), COORD_SPINE=os.path.join(T, "sp"))
sys.path.insert(0, HERE)
import server  # noqa: E402
import sponsor_bot  # noqa: E402
if CONTROL:
    sponsor_bot.LIBZYNC = False
    sponsor_bot.SMOKE_LIBZYNC = sponsor_bot.SMOKE_CORE
    print("CONTROL: the bot looks for bundle_<n>.json on a libzync coordinator -- the checks below MUST fail")

fails = 0
def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what); fails += 0 if cond else 1

open(os.path.join(BR, "leaf_10.bin"), "wb").write(b"LZLEAF02...")
open(os.path.join(BR, "bundle_11.json"), "w").write("{}")
served = lambda h: any(os.path.exists(f) for f in server._bridge_files(h))
check(server.LIBZYNC and served(10) and not served(11), "the coordinator (libzync) serves leaf_10.bin, not bundle_11.json")
for h in (10, 11, 12):
    check(sponsor_bot.has_bundle(h) == served(h),
          f"the bot's has_bundle({h}) = {sponsor_bot.has_bundle(h)} agrees with what the coordinator serves ({served(h)})")

loader = importlib.machinery.SourceFileLoader("hazync_worker", os.path.join(HERE, "hazync"))
spec = importlib.util.spec_from_loader("hazync_worker", loader)
worker = importlib.util.module_from_spec(spec)
loader.exec_module(worker)
check(worker.bundle_name(b"LZLEAF02\x00\x01", 1) == "leaf_1.bin", "a served LZLEAF bundle is stored as leaf_1.bin")
check(worker.bundle_name(b'{"witness": {}}', 1) == "bundle_1.json", "a JSON bundle is stored as bundle_1.json")

# 3. the pods' GPU smoke: a libzync board proves block 1 from the leaf bundle it ships, and passes only on a
#    RANGE-OK for block 1; the original board's smoke is unchanged
lz_boot, core_boot = sponsor_bot.boot_script("v9.9.9", libzync=True), sponsor_bot.boot_script("v9.9.9", libzync=False)
check("prove-range-bridge 1" in lz_boot and "verify-any range_1.bin 1" in lz_boot and "RANGE-OK lo=1 hi=1 " in lz_boot
      and "prove-block" not in lz_boot and "@SMOKE@" not in lz_boot,
      "a libzync pod proves block 1 from its leaf bundle and passes only on its RANGE-OK")
check("prove-block" in core_boot and "prove-range-bridge" not in core_boot and "/v9.9.9/" in core_boot,
      "the original board's pod smoke is unchanged")
check([os.path.basename(f) for f in sponsor_bot.boot_files("/d", libzync=True)] == ["boot.sh", "want.txt", "leaf_1.bin"]
      and [os.path.basename(f) for f in sponsor_bot.boot_files("/d", libzync=False)] == ["boot.sh", "want.txt"],
      "boot() copies leaf_1.bin to a pod on the libzync board only")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
