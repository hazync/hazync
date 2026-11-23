# nsys_facts.py <export.sqlite>: kernel shares and the process timeline (ns from the session start)
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
tabs = {r[0] for r in c.execute("select name from sqlite_master where type='table'")}
k = c.execute("select k.start, k.end, s.value from CUPTI_ACTIVITY_KIND_KERNEL k join StringIds s on s.id = k.shortName order by k.start").fetchall()
tot = sum(e - s for s, e, _ in k)
by = {}
for s, e, n in k: by.setdefault(n, [0, 0]); by[n][0] += e - s; by[n][1] += 1
print(f"kernels {len(k)}  total kernel time {tot/1e9:.3f} s")
for n, (t, cnt) in sorted(by.items(), key=lambda x: -x[1][0])[:15]:
    print(f"  {100*t/tot:6.2f}%  {t/1e9:9.3f} s  {cnt:7d}x  {n[:90]}")
# busy = union of kernel intervals
busy = 0; cur_s = cur_e = None
for s, e, _ in k:
    if cur_e is None or s > cur_e:
        if cur_e is not None: busy += cur_e - cur_s
        cur_s, cur_e = s, e
    else: cur_e = max(cur_e, e)
if cur_e is not None: busy += cur_e - cur_s
end = None
if "ANALYSIS_DETAILS" in tabs:
    try: end = c.execute("select duration from ANALYSIS_DETAILS").fetchone()[0]
    except Exception: end = None
rt = c.execute("select min(start) from CUPTI_ACTIVITY_KIND_RUNTIME").fetchone()[0] if "CUPTI_ACTIVITY_KIND_RUNTIME" in tabs else None
fe = next((s for s, e, n in k if "eval_check" in n or "piece" in n), None)
print(f"first CUDA runtime call {rt/1e9 if rt else float('nan'):.3f} s, first kernel {k[0][0]/1e9:.3f} s, first eval_check {fe/1e9 if fe else float('nan'):.3f} s, last kernel end {k[-1][1]/1e9:.3f} s, session {end/1e9 if end else float('nan'):.3f} s")
print(f"GPU busy (union of kernels) {busy/1e9:.3f} s = {100*busy/(k[-1][1]-k[0][0]):.1f}% of first..last kernel")
