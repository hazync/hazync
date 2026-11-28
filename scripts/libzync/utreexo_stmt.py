#!/usr/bin/env python3
"""The utreexo zAPP's statement from an engine execution's journal (risc0 serde words: u32 each, a byte per word in
[u8; N], u64 as two words, a Vec as its length then its items), printed as the zAPP host prints its expectation:
  utreexo_stmt.py base|tip <journal>  ->  height=<H> block=<hash, display order> leaves=<n> roots=<hex,...|empty>
The layouts are the guests' (prover/zyncapps/methods/utreexo BaseStatement, utreexo-tip TipStatement)."""
import struct, sys
kind, path = sys.argv[1], sys.argv[2]
b = open(path, "rb").read(); w = list(struct.unpack(f"<{len(b) // 4}I", b)); i = 0
def u32():
    global i; i += 1; return w[i - 1]
def u64():
    lo, hi = u32(), u32(); return lo | hi << 32
def b32():
    global i; i += 32; return bytes(w[i - 32:i])
def ids(): return [u32() for _ in range(8)]
u32(); ids()                                   # kind, self_id
if kind == "base":
    u32(); height = u32(); u32(); u32(); b32(); u32()       # network, height, to, runs, state_out, done
else:
    ids(); u32(); u32(); ids(); u32(); u32(); height = u32()  # base_id, network, base_height, base_core_id, steps, from, height
block = b32(); leaves = u64(); n = u32(); roots = [b32() for _ in range(n)]
rs = ",".join("empty" if r == bytes(32) else r.hex() for r in roots)
print(f"height={height} block={block[::-1].hex()} leaves={leaves} roots={rs}")
