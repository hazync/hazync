#!/usr/bin/env python3
"""test_utxo_snapshot.py <block json dir> <H> <state.json> -- end-to-end test of utxo_snapshot_check.py without a
node: writes the UTXO set at H as a Core v28 `dumptxoutset` file (encoder from Core's compressor.cpp), a stub
`bitcoin-cli` that answers getblockhash / getblockheader / getblock from the block files, runs the checker, and
expects UTXO_SNAPSHOT_MATCH against the guest's committed state. Then flips one coin's value and expects a mismatch.
"""
import json, os, struct, subprocess, sys, tempfile
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from utreexo_ref import parse_tx, header_hash, spendable, BIP30_UNSPENDABLE, GENESIS_TIME

d, H, state = sys.argv[1], int(sys.argv[2]), sys.argv[3]


def varint(n):
    out = bytearray()
    first = True
    while True:
        out.insert(0, (n & 0x7F) | (0 if first else 0x80))
        if n <= 0x7F:
            return bytes(out)
        n = (n >> 7) - 1
        first = False


def compact(n):
    return bytes([n]) if n < 0xFD else b"\xfd" + struct.pack("<H", n) if n <= 0xFFFF else b"\xfe" + struct.pack("<I", n)


def compress_amount(n):
    if n == 0:
        return 0
    e = 0
    while n % 10 == 0 and e < 9:
        n //= 10
        e += 1
    if e < 9:
        dgt = n % 10
        n //= 10
        return 1 + (n * 9 + dgt - 1) * 10 + e
    return 1 + (n - 1) * 10 + 9


def compress_script(s):
    if len(s) == 25 and s[:3] == b"\x76\xa9\x14" and s[23:] == b"\x88\xac":
        return b"\x00" + s[3:23]
    if len(s) == 23 and s[:2] == b"\xa9\x14" and s[22:] == b"\x87":
        return b"\x01" + s[2:22]
    if len(s) == 35 and s[0] == 33 and s[34] == 0xAC and s[1] in (2, 3):
        return s[1:34]
    if len(s) == 67 and s[0] == 65 and s[66] == 0xAC and s[1] == 4:
        return bytes([4 | (s[65] & 1)]) + s[2:34]
    return varint(len(s) + 6) + s


# The UTXO set at H, and the stub node's data, from the block files.
utxo, blocks, times = {}, {}, []
prev = None
for h in range(1, H + 1):
    j = json.load(open(f"{d}/block_{h}.json"))
    if h == 1:
        times.append(GENESIS_TIME[j["prev"]])
        blocks[j["prev"]] = {"height": 0, "mediantime": times[0], "tx": []}
    bh = header_hash(j)
    times.append(j["time"])
    raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
    txids = []
    for k, raw in enumerate(raws):
        txid, ins, outs = parse_tx(raw)
        txids.append(txid[::-1].hex())
        if k > 0:
            for i in ins:
                utxo.pop(i, None)
        if k == 0 and BIP30_UNSPENDABLE.get(h) == bh:
            continue
        for v, (value, spk) in enumerate(outs):
            if spendable(spk):
                utxo[(txid, v)] = (h, int(k == 0), value, spk)
    blocks[bh] = {"height": h, "mediantime": sorted(times[-11:])[len(times[-11:]) // 2], "tx": txids, "previousblockhash": j["prev"]}
    prev = bh
by_height = {b["height"]: k for k, b in blocks.items()}

tmp = tempfile.mkdtemp()
json.dump({"blocks": blocks, "by_height": by_height}, open(f"{tmp}/chain.json", "w"))
open(f"{tmp}/cli", "w").write(f"""#!/usr/bin/env python3
import json, sys
c = json.load(open("{tmp}/chain.json"))
cmd, a = sys.argv[1], sys.argv[2]
if cmd == "getblockhash": print(c["by_height"][a])
elif cmd == "getblockheader": print(json.dumps({{k: v for k, v in c["blocks"][a].items() if k != "tx"}}))
elif cmd == "getblock": print(json.dumps({{"tx": c["blocks"][a]["tx"]}}))
""")
os.chmod(f"{tmp}/cli", 0o755)


def write(path, coins):
    groups = {}
    for (txid, v), c in coins.items():
        groups.setdefault(txid, []).append((v, c))
    with open(path, "wb") as f:
        f.write(b"utxo\xff" + struct.pack("<H", 2) + b"\x0a\x03\xcf\x40" + bytes.fromhex(prev)[::-1] + struct.pack("<Q", len(coins)))
        for txid in sorted(groups):
            f.write(txid + compact(len(groups[txid])))
            for v, (h, cb, value, spk) in sorted(groups[txid]):
                f.write(compact(v) + varint(h * 2 + cb) + varint(compress_amount(value)) + compress_script(spk))


checker = os.path.join(os.path.dirname(os.path.abspath(__file__)), "utxo_snapshot_check.py")
write(f"{tmp}/good.dat", utxo)
r = subprocess.run([sys.executable, checker, f"{tmp}/good.dat", "--bitcoin-cli", f"{tmp}/cli", "--state", state], capture_output=True, text=True)
print(r.stdout.strip().splitlines()[0]); print(r.stdout.strip().splitlines()[-1])
bad = dict(utxo)
k0 = next(iter(bad))
h0, cb0, v0, s0 = bad[k0]
bad[k0] = (h0, cb0, v0 + 1, s0)
write(f"{tmp}/bad.dat", bad)
r2 = subprocess.run([sys.executable, checker, f"{tmp}/bad.dat", "--bitcoin-cli", f"{tmp}/cli", "--state", state], capture_output=True, text=True)
print("one coin's value + 1:", r2.stdout.strip().splitlines()[-1])
ok = r.returncode == 0 and r2.returncode == 1
print("TEST_UTXO_SNAPSHOT_OK" if ok else "TEST_UTXO_SNAPSHOT_FAIL")
sys.exit(0 if ok else 1)
