# A CoinPreimage from a zync-path journey: the origin tx's output `vout` (value, script), its txid, height, flags.
import json, sys, hashlib
j = json.load(open(sys.argv[1]))
raw = bytes.fromhex(j["origin"]["tx"])
def varint(b, i):
    n = b[i]
    if n < 0xfd: return n, i + 1
    k = {0xfd: 2, 0xfe: 4, 0xff: 8}[n]
    return int.from_bytes(b[i + 1:i + 1 + k], "little"), i + 1 + k
i = 4
seg = raw[4] == 0 and raw[5] == 1
if seg: i += 2
n, i = varint(raw, i)
for _ in range(n):
    i += 36; l, i = varint(raw, i); i += l + 4
n, i = varint(raw, i)
outs = []
for _ in range(n):
    v = int.from_bytes(raw[i:i + 8], "little"); i += 8
    l, i = varint(raw, i); outs.append((v, raw[i:i + l].hex())); i += l
if seg: raise SystemExit("segwit origin: strip witnesses for the txid (not needed for these early blocks)")
txid = hashlib.sha256(hashlib.sha256(raw).digest()).digest()[::-1].hex()
v, spk = outs[j["vout"]]
print(json.dumps({"txid": txid, "vout": j["vout"], "value": v, "script_pubkey": spk, "height": j["origin"]["height"],
                  "coinbase": j["origin"]["index"] == 0, "tracked": j["tracked"], "creation_mtp": j["creation_mtp"]}))
