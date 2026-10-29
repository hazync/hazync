"""tamper.py <hg dir> <collect log> <out root>: adversarial variants of chunk 0's batch hints (block 962,000).

Each scenario is a full copy of the hint dir with ONLY batch_0_388.bin changed. Kinds come from the
collect log's BATCHREC lines for chunk 0 (same call order as the hint records).
"""
import os, re, shutil, sys

P = 2**256 - 2**32 - 977
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
src, log, out = sys.argv[1], sys.argv[2], sys.argv[3]
F = "batch_0_388.bin"

recs, on = [], False
for line in open(log, errors="replace"):
    if "COLLECT for inputs" in line:
        if on: break
        on = "inputs 0..388" in line
        continue
    m = re.search(r"BATCHREC([0134]) ([0-9a-f]{320})", line)
    if on and m:
        b = bytes.fromhex(m.group(2))
        recs.append((int(m.group(1)), *(int.from_bytes(b[32 * k:32 * k + 32], "big") for k in range(5))))
blob = bytearray(open(f"{src}/{F}", "rb").read())
assert len(blob) == 68 * len(recs), (len(blob), len(recs))
flag = lambda i: int.from_bytes(blob[68 * i:68 * i + 4], "little")
R = lambda i: (int.from_bytes(blob[68 * i + 4:68 * i + 36], "big"), int.from_bytes(blob[68 * i + 36:68 * i + 68], "big"))

def put(b, i, f, x, y):
    b[68 * i:68 * i + 68] = f.to_bytes(4, "little") + x.to_bytes(32, "big") + y.to_bytes(32, "big")

def scenario(name, b, note):
    d = f"{out}/{name}"
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(src, d)
    open(f"{d}/{F}", "wb").write(b)
    print(f"{name}: {note}")

ecdsa_ok = next(i for i, r in enumerate(recs) if r[0] == 0 and flag(i) == 1)
schnorr_ok = next((i for i, r in enumerate(recs) if r[0] == 1 and flag(i) == 1), None)
bad = [i for i, r in enumerate(recs) if flag(i) == 0]
print(f"chunk 0: {len(recs)} records, kinds {sorted(set(r[0] for r in recs))}, flag-0 (failing) records {bad}")

scenario("base", bytes(blob), "unchanged")

b = bytearray(blob); x, y = R(ecdsa_ok); put(b, ecdsa_ok, 1, x ^ 1, y)
scenario("t1_bitflip", b, f"record {ecdsa_ok}: R.x low bit flipped (off-curve) -> expect VALID via fallback")

b = bytearray(blob); x, y = R(ecdsa_ok); put(b, ecdsa_ok, 1, x, P - y)
scenario("t2_negR", b, f"record {ecdsa_ok} (ECDSA, valid): R -> -R, same x so the acceptance test passes -> expect INVALID")

forged = None
for i in bad:
    kind, u1, u2, qx, qy, r = recs[i]
    if kind != 0: continue
    for cand in (r, r + N):           # x(R) mod n == r
        if cand >= P: continue
        rhs = (cand ** 3 + 7) % P
        yy = pow(rhs, (P + 1) // 4, P)
        if yy * yy % P == rhs:
            forged = (i, cand, yy); break
    if forged: break
if forged:
    i, fx, fy = forged
    b = bytearray(blob); put(b, i, 1, fx, fy)
    scenario("t3_forge", b, f"record {i} (a FAILING ECDSA check): flag 1 with R on the curve, x(R) = r -> expect INVALID")
else:
    print("t3_forge: SKIPPED, no failing ECDSA record in chunk 0 has a liftable r")

scenario("t4_truncate", bytes(blob[:68 * (len(recs) - 10)]), "last 10 records dropped -> expect VALID via fallback")

b = bytearray(blob)
for i in range(len(recs)): b[68 * i:68 * i + 4] = (0).to_bytes(4, "little")
scenario("t5_allzero", b, "every flag 0 -> expect VALID, every verify in libsecp")

if schnorr_ok is not None:
    b = bytearray(blob); x, y = R(schnorr_ok); put(b, schnorr_ok, 1, x, P - y)
    scenario("t6_schnorr_negR", b, f"record {schnorr_ok} (Schnorr): R -> -R, odd y -> expect VALID via fallback")
