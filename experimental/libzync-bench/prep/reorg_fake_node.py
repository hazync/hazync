#!/usr/bin/env python3
"""A fake bitcoin-cli for the end-to-end reorg test (rewind_test.sh case F): it serves REAL block files (prover/testdata
and reorg_branch.py's) as a node would, so prover/fetch_block_rpc.py builds them again exactly and prep preps them.

  HAZYNC_RPC_CLI="python3 reorg_fake_node.py <world.json>"      (fetch_block_rpc.py appends the method and params)

<world.json>: {"chain": [path of block_1.json, path of block_2.json, ...]}, the node's active chain from height 1; the
test rewrites it (atomically) to make the node reorg. Only what fetch_block_rpc.py asks: getblockchaininfo,
getblockhash, getblock <hash> 1|3, getblockheader <hash>.
"""
import hashlib, json, struct, sys

GENESIS = "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
GENESIS_TIME, GENESIS_BITS = 1231006505, 0x1d00ffff

def header_hash(j):
    h = (struct.pack("<I", j["version"] & 0xffffffff) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
         + struct.pack("<III", j["time"], j["bits"], j["nonce"]))
    return hashlib.sha256(hashlib.sha256(h).digest()).digest()[::-1].hex()

def btc(sats): return f"{sats // 10**8}.{sats % 10**8:08d}"

def main():
    world = json.load(open(sys.argv[1]))
    m, a = sys.argv[2], sys.argv[3:]
    blocks = [json.load(open(p)) for p in world["chain"]]
    hashes = [GENESIS] + [header_hash(b) for b in blocks]
    times = [GENESIS_TIME] + [b["time"] for b in blocks]
    by_hash = {h: i for i, h in enumerate(hashes)}
    def out(x): print(x if isinstance(x, str) else json.dumps(x)); sys.exit(0)
    def die(x): print(x, file=sys.stderr); sys.exit(1)
    if m == "getblockchaininfo": out({"blocks": len(blocks), "bestblockhash": hashes[-1]})
    if m == "getblockhash":
        h = int(a[0])
        out(hashes[h]) if 0 <= h < len(hashes) else die("error code: -8\nBlock height out of range")
    h = by_hash.get(a[0])
    if h is None: die("error code: -5\nBlock not found")
    mtp = sorted(times[max(0, h - 10):h + 1])[len(times[max(0, h - 10):h + 1]) // 2]
    if h == 0:
        hdr = {"hash": GENESIS, "height": 0, "version": 1, "time": GENESIS_TIME, "mediantime": GENESIS_TIME,
               "bits": "%08x" % GENESIS_BITS, "nonce": 2083236893}
        out(hdr if m == "getblockheader" else dict(hdr, tx=[]))
    b = blocks[h - 1]
    hdr = {"hash": hashes[h], "height": h, "version": b["version"], "time": b["time"], "mediantime": mtp,
           "bits": "%08x" % b["bits"], "nonce": b["nonce"], "previousblockhash": b["prev"], "merkleroot": b["merkle"]}
    if m == "getblockheader": out(hdr)
    if m != "getblock": die("fake: " + m)
    if a[1:] != ["3"]: out(dict(hdr, tx=["00" * 32] * (1 + len(b["txs"]))))
    # verbosity 3: the coinbase's hex, then each transaction's hex and every input's prevout; amounts written as exact
    # decimals (the fetcher parses them with Decimal)
    txs, marks = [{"hex": b["coinbase_hex"]}], {}
    for t in b["txs"]:
        vin = []
        for p in t["prevouts"]:
            k = f"@@{len(marks)}@@"
            marks[k] = btc(p["value"])
            vin.append({"prevout": {"height": p["coin_height"], "generated": bool(p["coin_is_coinbase"] & 1),
                                    "value": k, "scriptPubKey": {"hex": p["spk"]}}})
        txs.append({"hex": t["raw"], "vin": vin})
    s = json.dumps(dict(hdr, tx=txs))
    for k, v in marks.items(): s = s.replace(f'"{k}"', v)
    out(s)

if __name__ == "__main__":
    main()
