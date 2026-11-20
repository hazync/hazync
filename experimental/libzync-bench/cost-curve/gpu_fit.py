"""gpu_fit.py <raw/gpu> <raw/table_a.tsv> <raw/table_b.tsv>: the GPU run's measured times per block and per part, joined
with the execute-mode cycle counts of the SAME blocks (the real proofs printed the same LZ_STATS), and the least-squares
cycles -> card-time fits on image 46666b6f (clean v0.27.0 build, one RTX PRO 6000).
A (leaf, libzync-prove-range h h ... leaf): times.tsv = sig-hint collect (CPU), chunk proofs, aggregate collect +
  trie proof, total; aggregate proof = total - the rest.  B (700k/976k, modelled state): steps.tsv per process:
  libzync-hints, prove-chunk i, agg-chunks (whose log gives the trie proof's own seconds; aggregate = the rest)."""
import csv, re, sys
G, TA, TB = sys.argv[1:4]
def tab(p): return {int(r["height"]): {k: (int(v) if v.lstrip("-").isdigit() else v) for k, v in r.items()} for r in csv.DictReader(open(p), delimiter="\t")}
A, B = tab(TA), tab(TB)
steps = [l.rstrip("\n").split("\t") for l in open(f"{G}/steps.tsv")]
st = {s[0]: float(s[3]) for s in steps}
rows = []
for h in (170, 1000, 100000, 200000, 270000, 280000, 290000, 300000, 400000):
    t = open(f"{G}/o{h}/times.tsv").read().split()
    sig, ch, tr, tot = float(t[2]), float(t[3]), float(t[4]), float(t[5])
    stats = re.findall(r"LZ_STATS (\w+) h=\d+ segments=(\d+) total_cycles=(\d+) user_cycles=(\d+)", open(f"{G}/leaf_o{h}.log").read())
    seg = {k: sum(int(s[1]) for s in stats if s[0] == k) for k in ("chunk", "trie", "aggregate")}
    usr = sum(int(s[3]) for s in stats); pad = sum(int(s[2]) for s in stats)
    assert usr == A[h]["total_user"], (h, usr, A[h]["total_user"])   # the proof ran the cycles the execute run counted
    rows.append(dict(h=h, src="A", wall=st[f"leaf {h} o{h}"], tot=tot, cpu=sig, chunk_s=ch, chunk_seg=seg["chunk"],
                     trie_s=tr, trie_seg=seg["trie"], agg_s=tot - sig - ch - tr, agg_seg=seg["aggregate"], user=usr, pad=pad, ins=A[h]["ins"]))
for h in (700000, 976000):
    r = B[h]; al = open(f"{G}/b{h}/agg.log").read()
    trie_s = float(re.search(r"trie update PROVED in ([\d.]+)", al)[1])
    n = int(open(f"{G}/b{h}/nchunks").read())
    ch = sum(st[f"B {h} chunk {i}"] for i in range(n)); agg = st[f"B {h} agg-chunks"]
    cseg = sum(int(re.search(r"(\d+) segments at po2", open(f"{G}/b{h}/chunk_{i}.log").read())[1]) for i in range(n))
    aseg = int(re.search(r"(\d+) segments to prove", al)[1])
    assert cseg == r["chunk_seg"] and aseg == r["agg_seg"], (h, cseg, aseg)
    rows.append(dict(h=h, src="B", wall=st[f"B {h} hints"] + ch + agg, tot=st[f"B {h} hints"] + ch + agg, cpu=st[f"B {h} hints"],
                     chunk_s=ch, chunk_seg=cseg, trie_s=trie_s, trie_seg=r["trie_seg"], agg_s=agg - trie_s, agg_seg=aseg,
                     user=r["total_user"], pad=r["total_padded"], ins=r["ins"]))
print("height src | total s | CPU hints s | chunks s (seg) | trie s (seg) | aggregate s (seg) | user M | padded M | s/padded-Mcyc | s/user-Mcyc")
for r in rows:
    print(f"{r['h']:>7} {r['src']} | {r['tot']:6.1f} | {r['cpu']:5.1f} | {r['chunk_s']:6.1f} ({r['chunk_seg']:3d}) | {r['trie_s']:6.1f} ({r['trie_seg']:2d}) |"
          f" {r['agg_s']:6.1f} ({r['agg_seg']:2d}) | {r['user']/1e6:6.1f} | {r['pad']/1e6:6.1f} | {r['tot']/(r['pad']/1e6):5.3f} | {r['tot']/(r['user']/1e6):5.3f}")
def fit(xs, ys):
    n = len(xs); mx, my = sum(xs)/n, sum(ys)/n
    b = sum((x-mx)*(y-my) for x, y in zip(xs, ys)) / sum((x-mx)**2 for x in xs); a = my - b*mx
    return a, b, [y - (a + b*x) for x, y in zip(xs, ys)]
print()
for lab, sel in (("all 11 blocks", rows), ("small (A, <= 300k incl.)", [r for r in rows if r["h"] <= 300000]),
                 ("big (400k, 700k, 976k)", [r for r in rows if r["h"] >= 400000]),
                 ("with inputs (200k..976k)", [r for r in rows if r["h"] >= 200000])):
    a, b, res = fit([r["pad"]/1e6 for r in sel], [r["tot"] for r in sel])
    au, bu, resu = fit([r["user"]/1e6 for r in sel], [r["tot"] for r in sel])
    print(f"block wall, {lab}: {a:6.2f} s + {b:.4f} s/padded-Mcycle (residuals {min(res):+.1f}..{max(res):+.1f} s);"
          f"  {au:6.2f} s + {bu:.4f} s/user-Mcycle (residuals {min(resu):+.1f}..{max(resu):+.1f} s)")
# per part, GPU proving per segment (2^21 cycles)
for part in ("chunk", "trie", "agg"):
    sel = [r for r in rows if r[f"{part}_seg"] >= 5]
    a, b, res = fit([r[f"{part}_seg"] for r in sel], [r[f"{part}_s"] for r in sel])
    print(f"part {part:5s}: {a:6.2f} s + {b:.3f} s/segment  (= {b/2.097152:.4f} s/padded-Mcycle; {len(sel)} blocks with >=5 segments,"
          f" residuals {min(res):+.1f}..{max(res):+.1f} s)")
tot_s = sum(r["tot"] for r in rows); print(f"\nsum of the 11 block walls: {tot_s:.1f} s ({tot_s/3600:.3f} card-hours)")
