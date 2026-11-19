#!/usr/bin/env python3
"""forge_block.py <block_<h>.json> <coin txid display> <value> <coin height> <coin mtp> <out json>  -- NEGATIVE CONTROL

TRUSTED_STATE.md §11.1. Writes a copy of a regtest block with ONE extra transaction that spends <txid>:0, an OP_TRUE
(`51`) coin of <value> sat that does NOT exist on the chain (ts-anchor build --forge-extra put it in a forged anchor's
coin log and trie only), paying the same value to OP_TRUE. The merkle root is recomputed and the nonce re-ground for
regtest's target, so the header is valid; Bitcoin Core would reject the block (bad-txns-inputs-missingorspent).
"""
import json, os, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "scripts", "libzync"))
from utreexo_ref import parse_tx, dsha  # noqa: E402


def merkle(h):
    while len(h) > 1:
        if len(h) % 2:
            h.append(h[-1])
        h = [dsha(h[i] + h[i + 1]) for i in range(0, len(h), 2)]
    return h[0]


def main():
    src, txid_disp, value, height, mtp, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5]), sys.argv[6]
    j = json.load(open(src))
    raw = (b"\x01\x00\x00\x00" + b"\x01" + bytes.fromhex(txid_disp)[::-1] + (0).to_bytes(4, "little") + b"\x00"
           + b"\xff\xff\xff\xff" + b"\x01" + value.to_bytes(8, "little") + b"\x01\x51" + (0).to_bytes(4, "little"))
    j["txs"].append({"raw": raw.hex(), "prevouts": [{"value": value, "spk": "51", "coin_height": height,
                                                      "coin_is_coinbase": 0, "coin_mtp": mtp}]})
    ids = [parse_tx(bytes.fromhex(j["coinbase_hex"]))[0]] + [parse_tx(bytes.fromhex(t["raw"]))[0] for t in j["txs"]]
    j["merkle"] = merkle(ids)[::-1].hex()
    bits = j["bits"]
    target = (bits & 0x7fffff) << (8 * ((bits >> 24) - 3))
    for nonce in range(1 << 32):
        h = (j["version"].to_bytes(4, "little", signed=True) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
             + j["time"].to_bytes(4, "little") + bits.to_bytes(4, "little") + nonce.to_bytes(4, "little"))
        if int.from_bytes(dsha(h), "little") <= target:
            j["nonce"] = nonce
            break
    json.dump(j, open(out, "w"))
    print(f"forged block {j['height']}: +1 tx {parse_tx(raw)[0][::-1].hex()} spending {txid_disp}:0 ({value} sat), "
          f"{len(j['txs']) + 1} txs, merkle {j['merkle']}, nonce {j['nonce']}, hash {dsha(h)[::-1].hex()}")


if __name__ == "__main__":
    main()
