#!/usr/bin/env python3
"""side_inputs.py <block json dir> <H> <out prefix>  -- EXPERIMENTAL (docs/libzync/TRUSTED_STATE.md §3)

The three side inputs of a Trusted State anchor at H that a Core `dumptxoutset` snapshot does not carry, from the host's
block files 1..H (block_<h>.json). Whoever publishes an anchor runs this once (it needs the blocks); a node that loads
the anchor does not. Writes:

  <prefix>.headers     the 80-byte headers of blocks 0..H, oldest first (each coin's creation MTP; cum_work, the time
                       context and the block forest's hashes). Bound to the snapshot's base block by hashing alone.
  <prefix>.tracked     the BIP30-tracked coins unspent at H, `<txid display>:<vout>` per line (leaf flag bit 1,
                       UTREEXO_FORMAT.md §2a): zapp-assume's --tracked format
  <prefix>.repeatable  the tracked transactions with no UNSPENT output left at H, `<txid display>` per
                       line: the BIP30 tree's REPEATABLE slots (coinbase-smt bip30.rs), which no snapshot can show

The tracked rule is utreexo_ref.py's `Tracked` (an independent implementation of the guest's). Python stdlib only.
"""
import json, os, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "scripts", "libzync"))
from utreexo_ref import Tracked, parse_tx, spendable, header_hash, BIP30_UNSPENDABLE, dsha  # noqa: E402

# Genesis headers (version, merkle display, time, bits, nonce) by genesis hash (display order)
GENESIS = {
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f": (1, 1231006505, 0x1d00ffff, 2083236893),   # mainnet
    "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206": (1, 1296688602, 0x207fffff, 2),            # regtest
}
GENESIS_MERKLE = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"


def header_bytes(version, prev_display, merkle_display, time, bits, nonce):
    return (version.to_bytes(4, "little", signed=True) + bytes.fromhex(prev_display)[::-1]
            + bytes.fromhex(merkle_display)[::-1] + time.to_bytes(4, "little")
            + bits.to_bytes(4, "little") + nonce.to_bytes(4, "little"))


def main():
    d, H, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    hdrs = []
    trk = None
    live_tracked = set()          # tracked outpoints unspent so far
    tracked_txids = set()         # tracked transactions with at least one spendable output (the tree's slots)
    prev = None
    for h in range(1, H + 1):
        j = json.load(open(f"{d}/block_{h}.json"))
        if h == 1:
            g = j["prev"]
            assert g in GENESIS, f"block 1's parent {g} is not a known genesis"
            v, t, b, n = GENESIS[g]
            gh = header_bytes(v, "00" * 32, GENESIS_MERKLE, t, b, n)
            assert dsha(gh)[::-1].hex() == g, "genesis header does not hash to its id"
            hdrs.append(gh)
            trk = Tracked(g)
        else:
            assert j["prev"] == prev, f"block {h} does not link to block {h - 1}"
        hb = header_bytes(j["version"], j["prev"], j["merkle"], j["time"], j["bits"], j["nonce"])
        prev = header_hash(j)
        assert dsha(hb)[::-1].hex() == prev
        hdrs.append(hb)
        bip30 = BIP30_UNSPENDABLE.get(h) == prev
        raws = [bytes.fromhex(j["coinbase_hex"])] + [bytes.fromhex(t["raw"]) for t in j["txs"]]
        for k, raw in enumerate(raws):
            txid, ins, outs = parse_tx(raw)
            t = trk.tx(h, k, raw, txid, ins if k > 0 else [])
            if k > 0:
                live_tracked.difference_update(ins)
            if k == 0 and bip30:
                continue
            if t:
                sp = False
                for v, (_, spk) in enumerate(outs):
                    trk.created((txid, v))
                    if spendable(spk):
                        live_tracked.add((txid, v))
                        sp = True
                if sp:
                    tracked_txids.add(txid)
    with open(out + ".headers", "wb") as f:
        f.write(b"".join(hdrs))
    with open(out + ".tracked", "w") as f:
        f.write(f"# BIP30-tracked coins unspent at height {H} (experimental/trusted-state/side_inputs.py)\n")
        for (txid, v) in sorted(live_tracked):
            f.write(f"{txid[::-1].hex()}:{v}\n")
    live_txids = {t for (t, _) in live_tracked}
    rep = sorted(tracked_txids - live_txids)
    with open(out + ".repeatable", "w") as f:
        f.write(f"# BIP30-tracked transactions with no unspent output at height {H}: REPEATABLE slots (side_inputs.py)\n")
        for txid in rep:
            f.write(f"{txid[::-1].hex()}\n")
    print(f"side inputs at {H}: {len(hdrs)} headers, {len(live_tracked)} tracked live coins, {len(rep)} repeatable txids")


if __name__ == "__main__":
    main()
