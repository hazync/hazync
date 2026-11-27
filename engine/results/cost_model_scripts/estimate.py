# estimate.py <report.txt> <dump dir> [<measured.txt>]: each chunk's ENGINE cost estimated from the fitted per-input
# engine model (4.2211 x (base+sig+2*share) + 155.5 x legacy_pos + 49,147 x script_extra, + 4.79 M per chunk; fitted on
# the measured chunks of the OLD split), and the straggler it gives (max / mean). With measured.txt: the estimate's
# straggler beside the measured one, to show how far the estimator itself is off on the split that was measured.
import sys, collections, numpy as np
rep, d = sys.argv[1], sys.argv[2]
bounds = collections.defaultdict(dict); tot = {}
for l in open(rep):
    p = l.split()
    if len(p) > 4 and p[2] == 'chunk': lo, hi = p[4].strip('[)').split('..'); bounds[p[1]][int(p[3])] = (int(lo), int(hi))
m = collections.defaultdict(dict)
if len(sys.argv) > 3:
    for l in open(sys.argv[3]): b, c, v = l.split(); m[b][int(c.split('_')[1])] = int(v)
for b, bb in bounds.items():
    inp = np.array([list(map(int, l.split())) for l in open(f"{d}/{b}.inputs")], float)
    e = 4.2211 * (inp[:, 1] + 2 * inp[:, 3]) + 155.5 * inp[:, 4] + 49147 * inp[:, 5]
    est = [e[lo:hi].sum() + 4.79e6 for lo, hi in bb.values()]
    line = f"{b:10} chunks {len(est):2}  estimated engine {sum(est):>18,.0f}  est. straggler {max(est)/np.mean(est):5.2f}"
    if b in m and len(m[b]) == len(est):
        mv = list(m[b].values()); line += f"  | measured straggler {max(mv)/np.mean(mv):5.2f}  measured total {sum(mv):>18,}"
    print(line)
