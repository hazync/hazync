#!/usr/bin/env python3
"""Block files with HEADERS ONLY (no transactions) for the follower's logic tests, from a "<height> <80-byte header hex>"
file (headers_1_500.txt: mainnet 1..500). test_follow_logic.sh's stub prover and fake-bitcoin-cli read only the header
fields, so CI can run it without mainnet block files.
    mkblocks.py <headers file> <out dir>"""
import json, os, struct, sys
src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
for line in open(src):
    h, x = line.split()
    b = bytes.fromhex(x)
    assert len(b) == 80
    version, = struct.unpack('<i', b[:4])
    t, bits, nonce = struct.unpack('<III', b[68:80])
    json.dump({'height': int(h), 'version': version, 'prev': b[4:36][::-1].hex(), 'merkle': b[36:68][::-1].hex(),
               'time': t, 'bits': bits, 'nonce': nonce, 'coinbase_hex': '', 'txs': []}, open(f'{out}/block_{h}.json', 'w'))
