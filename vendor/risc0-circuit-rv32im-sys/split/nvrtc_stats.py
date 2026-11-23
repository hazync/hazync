#!/usr/bin/env python3
"""nvrtc_stats.py: compile eval_check translation units with NVRTC (device code only) and print ptxas's
registers / spill / stack per function, without a full CUDA toolkit or a GPU.

  nvrtc_stats.py <libnvrtc.so.12> <arch e.g. sm_120> <file.cu>... [-I dir]...

The host launcher at the end of a generated eval_check_split*.cu (after the '// Host launcher' line) and the
cuda_util.h include are stripped, since NVRTC compiles device code only. Flags follow build.rs: -rdc, -O3
(NVRTC's default), -std=c++17.
"""
import ctypes, re, sys, time

lib = ctypes.CDLL(sys.argv[1])
arch = sys.argv[2]
files, incs = [], []
a = sys.argv[3:]
i = 0
while i < len(a):
    if a[i] == "-I":
        incs.append(a[i + 1]); i += 2
    else:
        files.append(a[i]); i += 1


def chk(r, what, prog=None):
    if r != 0:
        msg = ctypes.c_char_p(lib.nvrtcGetErrorString(r)).value.decode()
        if prog is not None:
            n = ctypes.c_size_t()
            lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
            buf = ctypes.create_string_buffer(n.value)
            lib.nvrtcGetProgramLog(prog, buf)
            print(buf.value.decode()[-6000:])
        sys.exit(f"{what}: {msg}")


lib.nvrtcGetErrorString.restype = ctypes.c_char_p
for f in files:
    src = open(f).read()
    if "// Host launcher" in src:
        src = src[:src.index("// Host launcher")]
    src = src.replace('#include "cuda_util.h"', "").replace("#include <exception>", "").replace("#include <cstring>", "")
    src = src.replace("using namespace risc0::circuit::rv32im_v2::cuda;", "")
    if "eval_check_split" not in f and "eval_check_chain" not in f:
        pass
    prog = ctypes.c_void_p()
    chk(lib.nvrtcCreateProgram(ctypes.byref(prog), src.encode(), f.encode(), 0, None, None), "create")
    opts = [f"--gpu-architecture={arch}", "-rdc=true", "-std=c++17", "--ptxas-options=-v",
            "-diag-suppress=177,550,2922", "-default-device"] + [f"-I{d}" for d in incs]
    copts = (ctypes.c_char_p * len(opts))(*[o.encode() for o in opts])
    t0 = time.time()
    r = lib.nvrtcCompileProgram(prog, len(opts), copts)
    dt = time.time() - t0
    n = ctypes.c_size_t()
    lib.nvrtcGetProgramLogSize(prog, ctypes.byref(n))
    buf = ctypes.create_string_buffer(n.value)
    lib.nvrtcGetProgramLog(prog, buf)
    log = buf.value.decode()
    if r != 0:
        print(log[-8000:])
        sys.exit(f"{f}: compile failed")
    print(f"== {f} ({arch}, {dt:.0f} s)")
    fn = None
    for line in log.splitlines():
        m = re.search(r"Compiling entry function '(\w+)'|Function properties for (\w+)|Compiling function '(\w+)'", line)
        if m:
            fn = m.group(1) or m.group(2) or m.group(3)
        m = re.search(r"(\d+) bytes stack frame, (\d+) bytes spill stores, (\d+) bytes spill loads", line)
        if m and fn:
            print(f"  {fn[:60]:60s} stack {m.group(1):>6} spill_st {m.group(2):>6} spill_ld {m.group(3):>6}")
        m = re.search(r"Used (\d+) registers", line)
        if "ptxas" in line and ("Used" in line or "error" in line.lower()) and "-v" in sys.argv[0:0]: print(line)
        if m and fn:
            print(f"  {fn[:60]:60s} regs {m.group(1)}")
