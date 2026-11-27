# compare.py <report.txt> <measured.txt>: per block, per chunk predicted vs measured engine cost; the ratio's spread
# within a block (max/min of measured/predicted) and the straggler (max measured / mean measured) the split gives.
import sys, collections
pred = collections.defaultdict(dict); tot = {}
for l in open(sys.argv[1]):
    p = l.split()
    if len(p) > 3 and p[2] == 'chunk': pred[p[1]][int(p[3])] = (p[4], int(p[6]))
    elif len(p) > 3 and p[2] == 'inputs': tot[p[1]] = (int(p[3]), int(p[5]), int(p[7]))
meas = collections.defaultdict(dict)
for l in open(sys.argv[2]):
    b, c, v = l.split(); meas[b][int(c.split('_')[1])] = int(v)
for b in pred:
    n, k, t = tot[b]; m = meas.get(b, {})
    line = f"{b:10} inputs {n:6} chunks {k:2} predicted {t:>15,}"
    if len(m) == len(pred[b]):
        r = [m[i] / pred[b][i][1] for i in sorted(m)]
        mv = list(m.values()); line += f" | measured {sum(mv):>16,}  engine/model {sum(mv)/t:6.2f}  ratio spread {max(r)/min(r):6.2f}  straggler {max(mv)/(sum(mv)/len(mv)):5.2f}"
    else:
        line += f" | measured chunks {len(m)} (split differs)" if m else ""
    print(line)
