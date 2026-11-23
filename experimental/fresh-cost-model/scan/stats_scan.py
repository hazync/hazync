"""Scan A: getblockstats for every height -> CSV. Resumable. Usage: stats_scan.py OUT.csv TIP [WORKERS]"""
import csv, os, sys, threading, queue
from rpc import RPC

FIELDS = ["height", "txs", "ins", "outs", "total_size", "total_weight", "swtxs", "swtotal_size",
          "utxo_increase", "utxo_size_inc", "total_out", "totalfee", "subsidy", "time"]
out, tip = sys.argv[1], int(sys.argv[2])
workers = int(sys.argv[3]) if len(sys.argv) > 3 else 4
done = set()
if os.path.exists(out):
    with open(out) as f:
        for row in csv.DictReader(f):
            done.add(int(row["height"]))
todo = [h for h in range(tip + 1) if h not in done]
print(f"{len(done)} done, {len(todo)} to do", flush=True)
q, results, lock = queue.Queue(), queue.Queue(), threading.Lock()
for h in todo:
    q.put(h)

def work():
    rpc = RPC()
    while True:
        try:
            h = q.get_nowait()
        except queue.Empty:
            return
        s = rpc.call("getblockstats", h, FIELDS[1:])
        s["height"] = h
        results.put(s)

ts = [threading.Thread(target=work, daemon=True) for _ in range(workers)]
for t in ts:
    t.start()
new = not os.path.exists(out)
with open(out, "a", newline="") as f:
    w = csv.DictWriter(f, fieldnames=FIELDS)
    if new:
        w.writeheader()
    n = 0
    while n < len(todo):
        w.writerow(results.get())
        n += 1
        if n % 10000 == 0:
            f.flush()
            print(f"{n}/{len(todo)}", flush=True)
print("done", flush=True)
