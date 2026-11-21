"""parse.py <runA dir> <runB dir> <blockstats.tsv> <out dir>: the measured tables from the raw logs.

(A) leaf_<h>.log: LZ_STATS <part> h= segments= total_cycles= user_cycles= (one line per chunk, trie, aggregate),
    TIMED, STATE_MATCH; o<h>/times.tsv (sig-hint s, chunk s, ecmh/trie s, total s).
(B) agg_<h>.log: `chunk i [..) executed: N cycles` / `chunk i padded P (S segments)`, `trie update executed: N
    cycles, padded P (S segments)`, `block validation cycles N` / `block validation padded P (S segments)`, journal;
    hints_<h>.log TIMED. "padded" = the sum of each segment's 2^po2, what a prover pays for (= total_cycles in A).
Writes table_a.tsv, table_b.tsv (one row per block) and prints both."""
import glob, os, re, sys
A, B, BS, OUT = sys.argv[1:5]
stats = {}
for l in open(BS):
    p = l.split()
    if p[0].isdigit(): stats[int(p[0])] = dict(zip(["txs", "ins", "outs", "weight"], map(int, p[1:5])))
def timed(path):
    m = re.search(r"TIMED wall_s=([\d.]+) user_s=([\d.]+).*maxrss_kb=(\d+) rc=(\d+) load1=([\d.]+)", open(path, errors="replace").read())
    return dict(wall=float(m[1]), user=float(m[2]), rss_mb=int(m[3]) // 1024, rc=int(m[4]), load=float(m[5])) if m else None
COLS = ["height", "src", "nchunks", "chunk_user", "chunk_padded", "chunk_seg", "trie_user", "trie_padded", "trie_seg",
        "agg_user", "agg_padded", "agg_seg", "total_user", "total_padded", "total_seg", "wall_s", "rss_mb", "load1",
        "check", "txs", "ins", "outs", "weight"]
def row(h, src, ch, tr, ag, wall, rss, load, check):
    r = dict(height=h, src=src, nchunks=len(ch), chunk_user=sum(c[0] for c in ch), chunk_padded=sum(c[1] for c in ch),
             chunk_seg=sum(c[2] for c in ch), trie_user=tr[0], trie_padded=tr[1], trie_seg=tr[2],
             agg_user=ag[0], agg_padded=ag[1], agg_seg=ag[2], wall_s=wall, rss_mb=rss, load1=load, check=check)
    for k in ("user", "padded", "seg"): r[f"total_{k}"] = r[f"chunk_{k}"] + r[f"trie_{k}"] + r[f"agg_{k}"]
    r.update(stats.get(h, {}))
    return r
rows_a, rows_b = [], []
for f in glob.glob(f"{A}/leaf_*.log"):
    h = int(re.search(r"leaf_(\d+)", f)[1]); t = open(f, errors="replace").read()
    parts = {"chunk": [], "trie": [], "aggregate": []}
    for m in re.finditer(r"LZ_STATS (\w+) h=\d+ segments=(\d+) total_cycles=(\d+) user_cycles=(\d+)", t):
        parts[m[1]].append((int(m[4]), int(m[3]), int(m[2])))
    if not parts["aggregate"]: continue
    tm = timed(f) or {}
    st = re.search(r"STATE_(MATCH|MISMATCH)[^\n]*", t)
    rows_a.append(row(h, "A real state", parts["chunk"], parts["trie"][0] if parts["trie"] else (0, 0, 0), parts["aggregate"][0],
                      tm.get("wall"), tm.get("rss_mb"), tm.get("load"), st[0].split(" (")[0] if st else "?"))
for f in glob.glob(f"{B}/agg_*.log"):
    h = int(re.search(r"agg_(\d+)", f)[1]); t = open(f, errors="replace").read()
    ex = {int(m[1]): int(m[2]) for m in re.finditer(r"chunk +(\d+) \[\d+\.\.\d+\) executed: (\d+) cycles", t)}
    pd = {int(m[1]): (int(m[2]), int(m[3])) for m in re.finditer(r"chunk +(\d+) padded (\d+) \((\d+) segments\)", t)}
    ch = [(ex[i], pd[i][0], pd[i][1]) for i in sorted(ex)]
    m = re.search(r"trie update executed: (\d+) cycles, padded (\d+) \((\d+) segments\)", t)
    tr = (int(m[1]), int(m[2]), int(m[3])) if m else (0, 0, 0)
    a1 = re.search(r"block validation cycles +(\d+)", t); a2 = re.search(r"block validation padded +(\d+) \((\d+) segments\)", t)
    if not (a1 and a2): continue
    j = re.search(r"journal sha256 +([0-9a-f]+)", t)
    th, ta = timed(f"{B}/hints_{h}.log") or {}, timed(f) or {}
    rows_b.append(row(h, "B modelled state", ch, tr, (int(a1[1]), int(a2[1]), int(a2[2])),
                      round(th.get("wall", 0) + ta.get("wall", 0), 1), max(th.get("rss_mb", 0), ta.get("rss_mb", 0)),
                      ta.get("load"), "journal " + (j[1][:16] if j else "?")))
for name, rows in (("table_a.tsv", rows_a), ("table_b.tsv", rows_b)):
    rows.sort(key=lambda r: r["height"])
    with open(os.path.join(OUT, name), "w") as o:
        o.write("\t".join(COLS) + "\n")
        for r in rows: o.write("\t".join(str(r.get(c, "")) for c in COLS) + "\n")
    print(f"== {name}")
    print(open(os.path.join(OUT, name)).read())
