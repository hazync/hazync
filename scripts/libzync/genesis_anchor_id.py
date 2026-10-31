#!/usr/bin/env python3
"""genesis_anchor_id.py <mainnet|signet> <METHOD_ID hex> [state.json] -- the anchor_id a genesis-anchored libzync
proof must commit (SWIFTSYNC_PROOFS.md §5 step 3), computed from the spec text alone: the genesis ChainState's
field values, risc0's serde word encoding, then double SHA-256.

With a ChainState JSON from a genesis run (a sweep's state_<H>.json) it compares against that state's anchor_id
and self_id, and exits 1 on any difference. Python standard library only.
"""
import hashlib, json, struct, sys

KIND_CHAIN = 0xC4A10002
TAG_SMT_NODE = 0x11
SMT_DEPTH = 256
NETS = {  # genesis hash (display order), time, nBits
    "mainnet": ("000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f", 1231006505, 0x1D00FFFF),
    "signet": ("00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6", 1598918400, 0x1E0377AE),
}


def sha256(b):
    return hashlib.sha256(b).digest()


def block_proof(bits):
    """Core GetBlockProof: 2^256 / (target + 1)."""
    target = (bits & 0x007FFFFF) << (8 * ((bits >> 24) - 3))
    return (1 << 256) // (target + 1)


def empty_smt_root():
    e = bytes(32)
    for _ in range(SMT_DEPTH):
        e = sha256(bytes([TAG_SMT_NODE]) + e + e)
    return e


class Words:
    """risc0 serde: integers <= 32 bits and each byte of a fixed array take one u32 word; u64 two (low first);
    a list is its length then its elements; an Option is 0/1 then the value."""
    def __init__(self):
        self.w = []

    def u32(self, x):
        self.w.append(x & 0xFFFFFFFF)

    def u64(self, x):
        self.u32(x)
        self.u32(x >> 32)

    def bytes_fixed(self, b):
        for x in b:
            self.u32(x)

    def raw(self):
        return b"".join(struct.pack("<I", x) for x in self.w)


def main():
    net, mid = sys.argv[1], sys.argv[2]
    g_hash, g_time, g_bits = NETS[net]
    self_id = list(struct.unpack("<8I", bytes.fromhex(mid)))
    w = Words()
    w.u32(KIND_CHAIN)                                       # kind
    w.bytes_fixed(bytes.fromhex(g_hash)[::-1])              # tip_hash, internal byte order
    w.u32(1); w.u32(0)                                      # utxo_roots = [None]: one entry, empty
    w.u64(0)                                                # utxo_leaves
    w.bytes_fixed(block_proof(g_bits).to_bytes(32, "little"))  # cum_work
    w.u32(0)                                                # height
    w.u32(g_bits)                                           # prev_nbits
    w.u32(g_time)                                           # prev_time
    w.u32(g_time)                                           # epoch_start
    w.u32(1); w.u32(g_time)                                 # recent_times = [genesis time]
    w.bytes_fixed(bytes(32))                                # anchor_id
    for x in self_id:                                       # self_id = M
        w.u32(x)
    w.u32(0)                                                # ss_set
    for _ in range(16):                                     # ss_acc
        w.u32(0)
    w.bytes_fixed(bytes(32))                                # ss_hints
    w.bytes_fixed(empty_smt_root())                         # smt_root
    w.u32(g_time)                                           # max_time
    w.u32(0)                                                # profile
    aid = sha256(sha256(w.raw()))
    print(f"{net} M={mid}: genesis anchor_id {aid.hex()}")
    if len(sys.argv) > 3:
        st = json.load(open(sys.argv[3]))
        ok = bytes(st["anchor_id"]) == aid and st["self_id"] == self_id
        print("ANCHOR_MATCH: the state's anchor_id equals the spec's genesis anchor" if ok else
              f"ANCHOR_MISMATCH: state anchor_id {bytes(st['anchor_id']).hex()}, self_id {st['self_id']}")
        sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
