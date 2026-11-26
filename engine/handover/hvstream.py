#!/usr/bin/env python3
"""hvstream.py <vk.bin> <proof (uncompressed encoding)> <out.bin> [flip-byte-index]: the handover verifier guest's input,
RISC Zero's serde word stream of (Vec<u8> vk, Vec<u8> proof): a length word, then one word per byte. With a flip index,
that byte of the proof is XORed with 1 (a negative control)."""
import struct, sys

vk, proof, out = open(sys.argv[1], "rb").read(), bytearray(open(sys.argv[2], "rb").read()), sys.argv[3]
if len(sys.argv) > 4:
    proof[int(sys.argv[4])] ^= 1
w = lambda b: struct.pack("<I", len(b)) + b"".join(struct.pack("<I", x) for x in b)
open(out, "wb").write(w(vk) + w(bytes(proof)))
print(f"stream: vk {len(vk)} B, proof {len(proof)} B -> {out}")
