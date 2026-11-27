# mmrpath.py <headers "height hex80" file> <height>: zapp_headers::mmr::path (prover/zyncapps/methods/headers/src/mmr.rs),
# one 64-hex hash per line, lowest first, as `zyncapp headers-mmr-path` prints it
import hashlib, sys
def sha(b): return hashlib.sha256(b).digest()
leaf = lambda h: sha(b'\x00' + h)
node = lambda l, r: sha(b'\x01' + l + r)
hs = []
for l in open(sys.argv[1]):
    if not l.strip(): continue
    h, x = l.split()[:2]; assert int(h) == len(hs); b = bytes.fromhex(x); assert len(b) == 80
    if hs: assert b[4:36] == hs[-1]
    hs.append(sha(sha(b)))
want = int(sys.argv[2]); n = len(hs)
off = 0
for k in range(63, -1, -1):
    if not (n >> k) & 1: continue
    if want < off + (1 << k): break
    off += 1 << k
level = [leaf(h) for h in hs[off:off + (1 << k)]]; i = want - off; out = []
while len(level) > 1:
    out.append(level[i ^ 1]); level = [node(level[j], level[j + 1]) for j in range(0, len(level), 2)]; i >>= 1
for p in out: print(p.hex())
print(f"block {want} {hs[want][::-1].hex()}", file=sys.stderr)
