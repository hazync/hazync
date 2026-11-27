#!/usr/bin/env python3
"""run_chunks.py <hzengine> <keys dir> <out dir> <export dir>...: every chunk_NN.in of each export executed (no proof)
through ONE `hzengine serve`; each journal compared with the export's recorded old-image journal (chunk_NN.j) and
RISC Zero's (chunk_NN.r0journal)."""
import json, os, subprocess, sys, time, hashlib, glob
H, K, O = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(O, exist_ok=True)
p = subprocess.Popen([H, "serve", "--dir", K], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=open(f"{O}/serve.log", "w"), text=True)
assert json.loads(p.stdout.readline()).get("ready"), "serve did not come up"
bad = 0; n = 0
for ex in sys.argv[4:]:
    name = ex.rstrip("/").replace("/", "_").split("ex_")[-1]
    for c in sorted(glob.glob(f"{ex}/chunk_[0-9][0-9].in")):
        k = os.path.basename(c)[:-3]; j = f"{O}/{name}_{k}.j"
        t = time.time()
        p.stdin.write(json.dumps({"op": "execute", "input": c, "journal": j}) + "\n"); p.stdin.flush()
        r = json.loads(p.stdout.readline())
        jb = open(j, "rb").read() if os.path.exists(j) else b""
        old = open(f"{ex}/{k}.j", "rb").read() if os.path.exists(f"{ex}/{k}.j") else None
        r0 = open(f"{ex}/{k}.r0journal", "rb").read() if os.path.exists(f"{ex}/{k}.r0journal") else None
        same_old = old == jb; same_r0 = r0 == jb
        # serve's execute reply carries exit = null (the run ended normally if ok); a non-null exit must be 0
        n += 1; bad += not (r.get("ok") and r.get("exit") in (None, 0) and same_old)
        print(f"{name} {k}: ok={r.get('ok')} exit={r.get('exit')} {time.time()-t:.1f}s journal {len(jb)} B sha256 {hashlib.sha256(jb).hexdigest()[:16]} "
              f"== old image's: {same_old} == RISC Zero's: {same_r0}{'' if r.get('ok') else ' ERROR ' + str(r.get('error'))[:200]}", flush=True)
p.stdin.close(); p.wait()
print(f"RUN_CHUNKS {n - bad}/{n} executed OK with the old image's journal")
