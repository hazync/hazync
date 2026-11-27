#!/usr/bin/env python3
"""make_vectors.py <swiftsync-verify binary>: writes this directory's engine vectors from ../tests/data/engine200 (the
engine chain's proof of mainnet 1..200): the proof, its key, one-change copies of each, and the bundles (written by the
verifier's own `bundle` command, then changed). Deterministic: run it again and nothing changes."""
import os, shutil, struct, subprocess, sys
here = os.path.dirname(os.path.abspath(__file__))
src = os.path.join(here, "..", "tests", "data", "engine200")
v = os.path.abspath(sys.argv[1])
rd = lambda n: open(os.path.join(src, n), "rb").read()
def wr(n, b):
    open(os.path.join(here, n), "wb").write(b)
P, K = rd("proof.hze"), rd("vk.bin")
img = rd("image_id").decode().strip()
wr("mainnet_1_200_engine.hze", P)
wr("mainnet_1_200_engine.vk", K)
wr("mainnet_1_200_engine.image_id", (img + "\n").encode())
n = struct.unpack("<I", P[4:8])[0]
flip = lambda b, i: b[:i] + bytes([b[i] ^ 1]) + b[i + 1:]
wr("mainnet_1_200_engine_proof_flipped.hze", flip(P, 8 + 1000))
wr("mainnet_1_200_engine_journal_flipped.hze", flip(P, len(P) - 40))
wr("mainnet_1_200_engine_trailing_byte.hze", P + b"\0")
wr("mainnet_1_200_engine_truncated.hze", P[: 8 + n // 2])
wr("mainnet_1_200_engine_length_past_end.hze", P[:4] + struct.pack("<I", 0xFFFFFFFF) + P[8:])
wr("mainnet_1_200_engine_key_flipped.vk", flip(K, len(K) - 1))
out = os.path.join(here, "mainnet_1_200_engine.sspf")
subprocess.run([v, "--network", "mainnet", "--image", img, "--vk", os.path.join(here, "mainnet_1_200_engine.vk"), "bundle",
                os.path.join(here, "mainnet_1_200_engine.hze"), out], check=True, capture_output=True)
B = open(out, "rb").read()
wr("bundle_header_lies.sspf", B[:38] + struct.pack("<I", 201) + B[42:])   # stop_height 201
wr("bundle_v1.sspf", B[:4] + b"\x01" + B[5:])
print("vectors written")
