#!/usr/bin/env python3
"""parse.py <worst.py output ...>: one table row per guest session, with the paper's GPU fit applied.

ESTIMATED GPU time per session = 23.77 s + 0.9940 s per million PADDED cycles (one RTX PRO 6000, the paper's fit);
dollars at $2.49/h. Sessions of a block prove in parallel on separate cards; the per-block card time is the sum.
"""
import re, sys

FIX, PER_M, USD_H = 23.77, 0.9940, 2.49


def gpu_s(padded):
    return FIX + PER_M * padded / 1e6


rows, cases = [], {}
for f in sys.argv[1:]:
    cur = None
    for line in open(f, errors="replace"):
        m = re.match(r"CASE (\S+) (.*)", line)
        if m:
            cases[m.group(1)] = m.group(2).strip()
        m = re.match(r"GUEST (\S+) chunks=(\d+) verdict=(\S+) rc=(-?\d+) (.*)$", line)
        if m:
            rest = m.group(5)
            t = re.search(r"TIMED .*?load1_end=\S+", rest)
            cur = {"case": m.group(1), "chunks": int(m.group(2)), "verdict": m.group(3), "timed": t.group(0) if t else "",
                   "why": rest[t.end():].strip() if t else rest, "s": []}
            rows.append(cur)
        m = re.match(r"\s+LZ_STATS (\w+) segments=(\d+) total_cycles=(\d+) user_cycles=(\d+)", line)
        if m and cur:
            cur["s"].append((m.group(1), int(m.group(2)), int(m.group(3)), int(m.group(4))))

print("| case | chunks | guest | session | user cycles | padded cycles | segments | GPU s (EST.) |")
print("|---|---|---|---|---:|---:|---:|---:|")
for r in rows:
    tot_u = tot_p = 0
    card = 0.0
    for (k, seg, pad, user) in r["s"]:
        print(f"| {r['case']} | {r['chunks']} | {r['verdict']} | {k} | {user:,} | {pad:,} | {seg:,} | {gpu_s(pad):,.0f} |")
        tot_u += user; tot_p += pad; card += gpu_s(pad)
    big = max((p for (k, _, p, _) in r["s"] if k == "chunk"), default=0)
    rss = re.search(r"maxrss_kb=(\d+)", r["timed"])
    print(f"| **{r['case']}** | {r['chunks']} | {r['verdict']} | **all** | **{tot_u:,}** | **{tot_p:,}** | | **{card:,.0f} s = "
          f"${card / 3600 * USD_H:,.2f}**; largest chunk {big:,} padded = {gpu_s(big):,.0f} s; host peak RSS "
          f"{int(rss.group(1)) / 1e6 if rss else 0:.2f} GB |")
print()
for k, v in cases.items():
    print(f"- CASE {k}: {v}")
for r in rows:
    if r["why"]:
        print(f"- {r['case']} chunks={r['chunks']}: {r['why']}")
