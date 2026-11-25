#!/usr/bin/env python3
"""FEASIBILITY Q9 driver: the core-vector streams in /root/engine/cv through K long-lived hzengine serve processes (keys
loaded once each), each verdict journal compared with RISC Zero; a stream RISC Zero aborted must fail on the engine too."""
import glob, json, os, subprocess, sys, threading, time
E = "/root/engine-kit/engine/host/target/release/hzengine"; X = "/root/engine/cv"; K = int(sys.argv[1]) if len(sys.argv) > 1 else 4
streams = sorted(glob.glob(X + "/cv_*.in")); todo = list(streams); lock = threading.Lock(); res = {}
def worker(k):
    cores = f"{4 * k}-{4 * k + 3}"
    p = subprocess.Popen(["taskset", "-c", cores, E, "serve", "--dir", "/root/engine/keys-hz24"], stdin=subprocess.PIPE,
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    p.stdout.readline()
    while True:
        with lock:
            if not todo: break
            f = todo.pop(0)
        b = f[:-3]
        p.stdin.write(json.dumps({"op": "execute", "input": f, "journal": b + ".ovm"}) + "\n"); p.stdin.flush()
        r = json.loads(p.stdout.readline())
        with lock: res[b] = r
    p.stdin.close(); p.wait()
t = time.time(); th = [threading.Thread(target=worker, args=(k,)) for k in range(K)]
[x.start() for x in th]
while any(x.is_alive() for x in th):
    time.sleep(60); print(f"  {len(res)}/{len(streams)} at {time.time() - t:.0f} s", flush=True)
same = diff = ab_ok = ab_bad = extra = 0
for f in streams:
    b = f[:-3]; r = res[b]; ok = r.get("ok") and r.get("exit") in (0, None)
    if os.path.exists(b + ".abort"):
        if ok: ab_bad += 1; print("ENGINE ACCEPTED A RISC0 ABORT:", b, r)
        else: ab_ok += 1
    elif not ok: extra += 1; print("ENGINE ABORTED:", b, open(b + ".cases").read().strip(), "cases:", str(r)[:200])
    elif open(b + ".ovm", "rb").read() == open(b + ".r0journal", "rb").read(): same += 1
    else: diff += 1; print("VERDICTS DIFFER:", b)
cases = sum(int(open(f[:-3] + ".cases").read()) for f in streams if os.path.exists(f[:-3] + ".cases"))
print(f"engine {time.time() - t:.0f} s: {same} streams identical verdicts ({cases} cases in all streams with journals), {diff} differ; "
      f"RISC Zero aborts also refused {ab_ok}, accepted {ab_bad}; engine-only aborts {extra}")
print("CV_DONE")
