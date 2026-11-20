"""capture_pprof.py <watched .pb> <out prefix> -- <command...>

Runs the command and keeps a copy of EVERY profile it writes to <watched .pb>. risc0's executor writes
RISC0_PPROF_OUT at the end of each execution and the next execution overwrites it, so a host command
that executes several guest runs (agg-exec-fake: chunk(s), the coin-log collect run, the trie update,
the aggregate) leaves only the last one. Each new version is copied to <out prefix>.<n>.pb once its
size and mtime have been stable for 1 s (the executions are seconds to minutes apart). The command's
stdout/stderr pass through; the exit code is the command's.
"""
import os, shutil, subprocess, sys, time

watched, prefix = sys.argv[1], sys.argv[2]
cmd = sys.argv[sys.argv.index("--") + 1:]
if os.path.exists(watched):
    os.remove(watched)
p = subprocess.Popen(cmd)
seen, n, cand, since = None, 0, None, 0.0


def poll():
    global seen, n, cand, since
    try:
        st = os.stat(watched); sig = (st.st_mtime_ns, st.st_size)
    except FileNotFoundError:
        return
    if sig == seen:
        return
    if sig != cand:
        cand, since = sig, time.time()
        return
    if time.time() - since >= 1.0 or p.poll() is not None:
        out = f"{prefix}.{n}.pb"
        shutil.copyfile(watched, out)
        print(f"capture_pprof: {out} ({sig[1]} bytes)", file=sys.stderr, flush=True)
        seen, n = sig, n + 1


while p.poll() is None:
    poll(); time.sleep(0.2)
for _ in range(3):  # the last write: settle and copy
    poll(); time.sleep(0.2)
sys.exit(p.returncode)
