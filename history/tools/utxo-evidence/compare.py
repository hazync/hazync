#!/usr/bin/env python3
"""Compare a Hazync HZUTXO v1 dump and a verified Hazync proof against Bitcoin Core at one height.

Inputs (all files, so the run can be repeated and audited):
  verify.json   hazync-verify --json <spine.bin>          (the proof's committed state)
  dump          host dump-snapshot output                  (our coin list)
  core.json     bitcoin-cli gettxoutsetinfo muhash          (Core's UTXO set)
  header.json   bitcoin-cli getblockheader <hash at height> (Core's tip + chainwork)

Every check prints PASS or FAIL; the exit code is 0 only if every check PASSED.
MuHash follows Core's own test (test/functional/feature_utxo_set_hash.py at v31.1): per coin,
outpoint || u32le(height*2 + coinbase) || i64le(value) || compactsize(len(spk)) || spk.
"""
import hashlib
import json
import struct
import sys
from decimal import Decimal

from cryptography.hazmat.primitives.ciphers import Cipher, algorithms

MODULUS = 2**3072 - 1103717


def num3072(key32):
    # ChaCha20 keystream, zero nonce, block counter 0..5 = 384 bytes. `cryptography` takes a 16-byte
    # nonce whose first 4 bytes are the little-endian block counter (RFC 7539 layout).
    enc = Cipher(algorithms.ChaCha20(key32, bytes(16)), mode=None).encryptor()
    return int.from_bytes(enc.update(bytes(384)), "little")


def compact_size(n):
    if n < 0xFD:
        return bytes([n])
    if n <= 0xFFFF:
        return b"\xfd" + struct.pack("<H", n)
    if n <= 0xFFFFFFFF:
        return b"\xfe" + struct.pack("<I", n)
    return b"\xff" + struct.pack("<Q", n)


def read_dump(path):
    b = open(path, "rb").read()
    if b[:8] != b"HZUTXO\0\0":
        sys.exit("not an HZUTXO dump")
    version, height, count = struct.unpack_from("<IIQ", b, 8)
    off, coins = 24, []
    for _ in range(count):
        txid = b[off:off + 32]
        vout, value, h, cb, mtp, pos, slen = struct.unpack_from("<IQIBIII", b, off + 32)
        off += 32 + 4 + 8 + 4 + 1 + 4 + 4 + 4
        spk = b[off:off + slen]
        if len(spk) != slen:
            sys.exit("dump truncated inside a coin")
        off += slen
        coins.append((txid, vout, value, h, cb, pos, spk))
    return version, height, count, coins, len(b) - off


def main():
    verify, dump_path, core_path, header_path = sys.argv[1:5]
    v = json.load(open(verify))
    core = json.load(open(core_path), parse_float=Decimal)
    hdr = json.load(open(header_path))
    version, d_height, d_count, coins, trailing = read_dump(dump_path)

    total = sum(c[2] for c in coins)
    mu = 1
    for (txid, vout, value, h, cb, pos, spk) in coins:
        ser = txid + struct.pack("<I", vout) + struct.pack("<I", h * 2 + cb)
        ser += struct.pack("<q", value) + compact_size(len(spk)) + spk
        mu = (mu * num3072(hashlib.sha256(ser).digest())) % MODULUS
    our_muhash = hashlib.sha256(mu.to_bytes(384, "little")).digest()[::-1].hex()
    core_sats = int(core["total_amount"] * 100_000_000)

    rows = [
        ("dump parses to its last byte", trailing, 0),
        ("dump version", version, 1),
        ("height: proof = Core", v["height"], core["height"]),
        ("height: dump = proof", d_height, v["height"]),
        ("tip hash: proof = Core", v["tip_hash"], hdr["hash"]),
        ("tip hash: Core header = Core UTXO set", hdr["hash"], core["bestblock"]),
        ("chain work: proof = Core", v["cumulative_work"], int(hdr["chainwork"], 16)),
        ("coin count: proof = dump", v["utxo_leaves"], d_count),
        ("coin count: dump = Core", d_count, core["txouts"]),
        ("total amount (sats): dump = Core", total, core_sats),
        ("MuHash of the whole set: dump = Core", our_muhash, core["muhash"]),
    ]
    ok = True
    for name, a, b in rows:
        good = a == b
        ok &= good
        print(f"{'PASS' if good else 'FAIL'}  {name}: {a}" + ("" if good else f"  !=  {b}"))
    print("ALL PASS" if ok else "SOMETHING FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
