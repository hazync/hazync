#!/usr/bin/env python3
"""undo_ref.py: a block's undo hash (the per-block undo-data commitment, format U0, #729), written from
docs/libzync/UNDO.md alone and sharing no code with the guest, the host or the verifier. Python standard library only.

  undo_ref.py <block.json>...                 print `undo_ref: block <h> inputs <n> hash <hex> u0_bytes <b>` per block
  undo_ref.py check <guest log> <block.json>  the guest's `undo: block <h> inputs <n> hash <hex>` line for that block
                                              must equal this file's; prints UNDO_REF_MATCH or UNDO_REF_MISMATCH (exit 1)

Per input of every non-coinbase transaction, in block order (the block file's `txs[].prevouts[]`, the host's JSON):
  record = code u32 LE (coin height * 2 + coinbase) | amount u64 LE | len(scriptPubKey) u32 LE | scriptPubKey
  digest = SHA-256(record)
  undo_hash = SHA-256(digest_1 | ... | digest_n)        (SHA-256 of nothing for a block without inputs)
`coin_is_coinbase` in the block file carries the BIP30-tracked flag in bit 1; the record takes bit 0 only.
"""
import hashlib, json, re, struct, sys


def undo_hash(j):
    """(undo hash, number of inputs, total record bytes) of a block file."""
    ds, nbytes, n = b"", 0, 0
    for t in j["txs"]:
        for p in t["prevouts"]:
            spk = bytes.fromhex(p["spk"])
            rec = struct.pack("<IQI", p["coin_height"] * 2 + (p["coin_is_coinbase"] & 1), p["value"], len(spk)) + spk
            ds += hashlib.sha256(rec).digest()
            nbytes += len(rec)
            n += 1
    return hashlib.sha256(ds).digest(), n, nbytes


def check(log_path, block_path):
    j = json.load(open(block_path))
    want, n, _ = undo_hash(j)
    pat = re.compile(r"undo: block %d inputs (\d+) hash ([0-9a-f]{64})" % j["height"])
    got = [m.groups() for m in pat.finditer(open(log_path, errors="replace").read())]
    if not got:
        print(f"UNDO_REF_MISMATCH block {j['height']}: no guest undo line in {log_path}")
        sys.exit(1)
    ok = all(int(k) == n and h == want.hex() for k, h in got)
    print(f"{'UNDO_REF_MATCH' if ok else 'UNDO_REF_MISMATCH'} block {j['height']} inputs {n} ref {want.hex()} guest {got}")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if sys.argv[1:2] == ["check"] and len(sys.argv) == 4:
        check(sys.argv[2], sys.argv[3])
    elif len(sys.argv) > 1 and sys.argv[1] != "check":
        for f in sys.argv[1:]:
            j = json.load(open(f))
            u, n, b = undo_hash(j)
            print(f"undo_ref: block {j['height']} inputs {n} hash {u.hex()} u0_bytes {b}")
    else:
        sys.exit(__doc__)
