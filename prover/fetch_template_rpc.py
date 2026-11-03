#!/usr/bin/env python3
# txlog Phase A (experimental/libzync-bench/txproofs/README.md §7): the node's block template for height tip+1 as a
# block fixture (fetch_block_rpc.py's schema), so the host proves SPECULATIVE chunks for it before the block exists:
#
#   python3 fetch_template_rpc.py <out.json> [max txs]
#   HAZYNC_SPECULATIVE=1 HAZYNC_BLOCK=<out.json> host <chunk mode ...>     (chunks commit zero hash and time)
#
# Read-only: getblocktemplate, gettxout (mempool-aware), getblockheader, getblockhash. Same RPC settings as
# fetch_block_rpc.py. The header fields a chunk does not use are placeholders (merkle zero, a dummy coinbase): a
# speculative chunk is only ever bound by a txlog aggregate to the real block, whose header the aggregate checks.
# Inputs spending a mempool parent get coin_height = tip+1 (the chunk then marks them "this height"); the aggregate
# finds them in the block if the parent is mined in the same block, and the zeroconf zyncAPP refuses them.
import json, sys
from fetch_block_rpc import rpc, sats


def main():
    out_path = sys.argv[1]
    cap = int(sys.argv[2]) if len(sys.argv) > 2 else 10**9
    t = rpc("getblocktemplate", [{"rules": ["segwit"]}])
    H = t["height"]
    tip = t["previousblockhash"]
    mtp_at = {}

    def block_mtp(hh):
        if hh < 0:
            return 0
        if hh not in mtp_at:
            mtp_at[hh] = rpc("getblockheader", [rpc("getblockhash", [hh])])["mediantime"]
        return mtp_at[hh]

    recent_times, ph = [], tip
    for _ in range(11):
        pj = rpc("getblockheader", [ph])
        recent_times.append(pj["time"])
        ph = pj.get("previousblockhash")
        if not ph:
            break
    recent_times.reverse()
    prev = rpc("getblockheader", [tip])
    epoch_first = ((H - 1) // 2016) * 2016
    eb = rpc("getblockheader", [rpc("getblockhash", [epoch_first])])
    # A minimal coinbase (BIP34 height push), never proved: the speculative chunks cover the other transactions only.
    hpush = H.to_bytes((H.bit_length() + 8) // 8, "little")
    sig = bytes([len(hpush)]) + hpush
    cb = (b"\x01\x00\x00\x00" + b"\x01" + b"\x00" * 32 + b"\xff\xff\xff\xff" + bytes([len(sig)]) + sig
          + b"\xff\xff\xff\xff" + b"\x01" + (0).to_bytes(8, "little") + b"\x01\x6a" + b"\x00" * 4).hex()
    out = {"height": H, "version": t["version"], "time": t["curtime"], "bits": int(t["bits"], 16), "nonce": 0,
           "prev": tip, "merkle": "00" * 32, "coinbase_hex": cb, "recent_times": recent_times,
           "epoch_start": eb["time"], "prev_time": prev["time"], "prev_bits": int(prev["bits"], 16),
           "speculative": True, "txs": []}
    for x in t["transactions"][:cap]:
        d = rpc("decoderawtransaction", [x["data"]])
        prevs = []
        for v in d["vin"]:
            o = rpc("gettxout", [v["txid"], v["vout"], True])
            if o is None:
                raise SystemExit(f"{x['txid']}: input {v['txid']}:{v['vout']} is not unspent (template changed?)")
            ch = H - o["confirmations"] + 1 if o["confirmations"] > 0 else H
            prevs.append({"value": sats(o["value"]), "spk": o["scriptPubKey"]["hex"], "coin_height": ch,
                          "coin_is_coinbase": 1 if o.get("coinbase") else 0, "coin_mtp": block_mtp(ch - 1)})
        out["txs"].append({"raw": x["data"], "prevouts": prevs})
    json.dump(out, open(out_path, "w"))
    print(f"template for height {H} on {tip[:16]}..: {len(out['txs'])} txs saved to {out_path}")


if __name__ == "__main__":
    main()
