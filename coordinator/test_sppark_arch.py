#!/usr/bin/env python3
"""sppark must compile kernels for Pascal, or a Pascal card has nothing to run (hazync#631).

⛔ THE FAILURE THIS GUARDS IS SILENT AT BUILD TIME AND BAFFLING AT RUN TIME. sppark's build.rs
asks nvcc for sm_70, sm_80, sm_120 and PTX for compute_80/compute_100. A GTX 1050 Ti is sm_61:
no cubin matches it, and PTX JIT only ever compiles FORWARD, so compute_80 PTX cannot be lowered
to sm_61. The build succeeds, the binary runs, sppark even SELECTS the card -- and then the first
kernel launch surfaces through the next cudaGetLastError() as

    Failure during zk_shift: cudaGetLastError()@sppark/ntt/ntt.cuh:97 failed: "operation not supported"

which names neither the architecture nor the kernel, and reads like a driver fault.

📏 MEASURED 2026-10-06 on a real GTX 1050 Ti. It is also why two earlier fixes were not enough:
lowering the DEVICE FILTER (HAZYNC_SPPARK_MIN_MAJOR=6) lets sppark pick the card up, and the
memory-pool fix (fix/631-nopool) reproduces the error byte for byte.

⚠ THE REAL RISK IS A VENDOR RE-SYNC. vendor/sppark is a vendored copy; pulling upstream again
would drop this flag without a word, and nothing else in the tree would notice.

    test_sppark_arch.py            # the flag must be requested, and guarded
    test_sppark_arch.py --control  # with the flag removed, this test MUST fail
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(os.path.dirname(HERE), "vendor", "sppark", "src", "build.rs")

CONTROL = "--control" in sys.argv
fails = []


def check(ok, what):
    print(("  ok   " if ok else "  FAIL ") + what)
    if not ok:
        fails.append(what)


src = open(SRC, encoding="utf-8").read()

if CONTROL:
    # ⚠ Remove exactly what the fix adds, and nothing else, so a pass here means the test is
    # reading the flag rather than something that merely travels with it.
    src = src.replace('.flag("arch=compute_61,code=sm_61")', '.flag("arch=compute_80,code=sm_80")')

check('arch=compute_61,code=sm_61' in src,
      "⛔ sm_61 is requested — without it a Pascal card has no kernel image at all")

# ⚠ GUARDED, NOT UNCONDITIONAL. CUDA 13 dropped Pascal, so an unguarded flag would break the
# build on a newer toolkit instead of merely skipping the architecture.
guarded = 'is_cuda_flag_supported(&nvcc, "-arch=sm_61")' in src
check(guarded, "⚠ and it is behind is_cuda_flag_supported, so CUDA 13 skips it rather than failing")

# ⚠ The lowest cubin must actually be the one we think it is. If upstream ever lowers its own
# floor this test should stop claiming credit for it, and if it RAISES the floor we want to know.
check('arch=compute_70,code=sm_70' in src,
      "the sm_70 baseline is still present (the architecture directly above Pascal)")

# ⛔ PTX ONLY JITS FORWARD. compute_80 PTX is embedded, and it is precisely what cannot rescue a
# Pascal card -- recording the fact here so nobody concludes the PTX makes sm_61 unnecessary.
# ⚠ The quotes are ESCAPED in the Rust source (code=\"compute_80,sm_80\"), so the needle must
# carry the backslashes too — searching for the unescaped form silently matches nothing.
check(r'arch=compute_80,code=\"compute_80,sm_80\"' in src,
      "⚠ compute_80 PTX is embedded — forward-JIT only, so it can NEVER serve sm_61")

print()
if CONTROL:
    if fails:
        print(f"PASS (control): {len(fails)} check(s) failed with sm_61 removed — the test can fail")
        sys.exit(0)
    print("FAIL (control): sm_61 was removed and every check still passed")
    sys.exit(1)
if fails:
    print(f"FAIL {len(fails)}: " + "; ".join(fails))
    sys.exit(1)
print("PASS (real)")
