"""Combine Scan A (exact per-height totals) and Scan B (sampled per-input work) into
whole-chain totals and a per-tip-block profile.  Usage: aggregate.py stats.csv detail.jsonl TIP"""
import csv, json, sys, statistics
from collections import defaultdict

stats_f, detail_f, tip = sys.argv[1], sys.argv[2], int(sys.argv[3])
ERA = 10_000
# which exact Scan-A total drives each sampled metric
DRIVER = defaultdict(lambda: "ins")
for k in ("sha_c_txid", "sha_c_wtxid"):
    DRIVER[k] = "total_size"
DRIVER["sha_c_merkle"] = "txs"; DRIVER["sha_c_header"] = "blocks"
DRIVER["utxo_create"] = "outs"; DRIVER["outputs_unspendable"] = "outs"

era_tot = defaultdict(lambda: defaultdict(int))
exact = defaultdict(int)
for r in csv.DictReader(open(stats_f)):
    h = int(r["height"])
    if h > tip:
        continue
    e = h // ERA
    for k in ("txs", "ins", "outs", "total_size", "total_weight", "utxo_increase", "utxo_size_inc"):
        v = int(r[k]); era_tot[e][k] += v; exact[k] += v
    era_tot[e]["blocks"] += 1; exact["blocks"] += 1

samples = [json.loads(l) for l in open(detail_f)]
errors = [s for s in samples if "error" in s]
samples = [s for s in samples if "error" not in s]
sampled = [s for s in samples if s["height"] % 250 == 0]
recent = sorted((s for s in samples if s["height"] > tip - 1008), key=lambda s: s["height"])
# a sampled block's own "drivers", in Scan-A vocabulary
def drivers(s):
    return {"ins": s.get("inputs", 0), "total_size": s["size"], "txs": s["txs"],
            "outs": s.get("utxo_create", 0) + s.get("outputs_unspendable", 0), "blocks": 1}
metrics = sorted({k for s in samples for k in s if k not in ("height", "txs", "size", "weight", "inputs")})
era_s = defaultdict(lambda: defaultdict(int)); era_d = defaultdict(lambda: defaultdict(int))
for s in sampled:
    e = s["height"] // ERA
    for k in metrics:
        era_s[e][k] += s.get(k, 0)
    for k, v in drivers(s).items():
        era_d[e][k] += v
chain = defaultdict(float)
for e, tot in era_tot.items():
    for k in metrics:
        d = DRIVER[k]
        if era_d[e][d]:
            chain[k] += era_s[e][k] / era_d[e][d] * tot[d]

out = {"tip": tip, "exact": dict(exact), "chain_est": {k: round(v) for k, v in chain.items()},
       "samples": len(sampled), "recent_blocks": len(recent), "scan_errors": len(errors)}
prof = {}
for k in ["txs", "size", "weight", "inputs"] + metrics:
    xs = [s.get(k, 0) for s in recent]
    if not xs or not any(xs):
        continue
    xs_sorted = sorted(xs)
    prof[k] = {"mean": round(statistics.mean(xs)), "p50": xs_sorted[len(xs) // 2],
               "p95": xs_sorted[int(len(xs) * 0.95)], "max": xs_sorted[-1]}
out["tip_block_profile"] = prof
json.dump(out, sys.stdout, indent=1)
