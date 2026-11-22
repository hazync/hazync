#!/usr/bin/env python3
"""coinbase_only_block.py <block_<h>.json> <out json>  -- EXPERIMENTAL (TRUSTED_STATE.md §14.2)

A copy of a regtest block with every transaction but the coinbase dropped, so it spends nothing: it can be proved on a
SYNTHETIC anchor (ts-anchor synth on the same headers) whose coins are not the chain's. The coinbase pays the subsidy
alone (the fees of the dropped transactions are gone) and its witness commitment is redone for a coinbase-only block
(witness root 0^32, the coinbase's own 32-byte witness nonce); the merkle root and the nonce are redone as
forge_block.py does. A valid block on that header chain; the header hash changes, so it is block h of a fork at h.
Only for blocks whose coinbase is segwit-serialised with one witness item, outputs [payout, OP_RETURN commitment].
"""
import json, os, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "scripts", "libzync"))
from utreexo_ref import parse_tx, dsha  # noqa: E402


def main():
    src, out = sys.argv[1], sys.argv[2]
    j = json.load(open(src))
    h = j["height"]
    cb = bytes.fromhex(j["coinbase_hex"])
    # version(4) marker flag(2) nin(1) prevout(36) script(len+…) seq(4) nout(1) out0 out1 witness locktime
    assert cb[4:6] == b"\x00\x01" and cb[6] == 1, "a segwit coinbase with one input"
    p = 7 + 36
    sl = cb[p]; p += 1 + sl + 4
    assert cb[p] == 2, "two outputs"; p += 1
    v0 = p; p += 8; l0 = cb[p]; p += 1 + l0
    v1 = p; p += 8; l1 = cb[p]; s1 = p + 1; p += 1 + l1
    assert l1 == 38 and cb[s1:s1 + 6] == bytes.fromhex("6a24aa21a9ed"), "output 1 is the witness commitment"
    assert cb[p] == 1 and cb[p + 1] == 32, "one 32-byte witness item (the nonce)"
    nonce_w = cb[p + 2:p + 34]
    subsidy = 50 * 100_000_000 >> (h // 150)  # regtest halving interval 150
    b = bytearray(cb)
    b[v0:v0 + 8] = subsidy.to_bytes(8, "little")
    b[s1 + 6:s1 + 38] = dsha(bytes(32) + nonce_w)  # commitment = dsha(witness root ‖ nonce), witness root of [coinbase] = 0^32
    j["coinbase_hex"] = bytes(b).hex()
    j["txs"] = []
    j["merkle"] = parse_tx(bytes(b))[0][::-1].hex()
    bits = j["bits"]
    target = (bits & 0x7fffff) << (8 * ((bits >> 24) - 3))
    for nonce in range(1 << 32):
        hd = (j["version"].to_bytes(4, "little", signed=True) + bytes.fromhex(j["prev"])[::-1] + bytes.fromhex(j["merkle"])[::-1]
              + j["time"].to_bytes(4, "little") + bits.to_bytes(4, "little") + nonce.to_bytes(4, "little"))
        if int.from_bytes(dsha(hd), "little") <= target:
            j["nonce"] = nonce
            break
    json.dump(j, open(out, "w"))
    print(f"coinbase-only block {h}: subsidy {subsidy}, merkle {j['merkle']}, nonce {j['nonce']}, hash {dsha(hd)[::-1].hex()}")


if __name__ == "__main__":
    main()
