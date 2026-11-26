#!/usr/bin/env python3
"""hdrtool.py: inputs and checks for the header zyncAPP on the engine (FEASIBILITY Q20).

  fetch <first> <last> <out.txt>        "height hex80" lines from the local node (bitcoin-cli; BITCOIN_CLI, BITCOIN_DATADIR)
  stream <headers.txt> <out.bin>        the guest's input from genesis, byte for byte what the zAPP host's headers-exec
                                        writes (ExecutorEnv: write(&empty HState), write(&n), write_slice(&words)) in
                                        RISC Zero's serde word format (u8 = one word, i64 = low then high word,
                                        a Vec = its length then its items, an array = its items)
  check <journal.bin> <headers.txt>     decode the journal (input digest, end HState) and check it against the node's
                                        getblockheader (hash, chainwork) and an independent MMR of the file's hashes
"""
import hashlib, json, os, struct, subprocess, sys

CLI = [os.environ.get("BITCOIN_CLI", "bitcoin-cli"), "-datadir=" + os.environ.get("BITCOIN_DATADIR", os.path.expanduser("~/.bitcoin"))]


def cli(*a):
    return subprocess.run(CLI + [str(x) for x in a], capture_output=True, text=True, check=True).stdout.strip()


def dsha(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def read_headers(p):
    out = []
    for l in open(p):
        if l.strip():
            h, x = l.split()
            out.append((int(h), bytes.fromhex(x)))
    return out


def words(*ws):
    return b"".join(struct.pack("<I", w & 0xFFFFFFFF) for w in ws)


def empty_state():
    # HState { height: -1, hash: [0;32], work: [0;32], bits: 0, time: 0, epoch_first_time: 0, recent_times: [],
    #          max_time: 0, mmr_peaks: [] }
    return (words(0xFFFFFFFF, 0xFFFFFFFF) + words(*[0] * 32) + words(*[0] * 32) + words(0, 0, 0) + words(0)
            + words(0) + words(0))


def mmr_append(peaks, n, h):
    # mmr.rs (prover/zyncapps/methods/headers) re-implemented from its spec: leaf = sha256(0x00 || h), node = sha256(0x01 || l || r); peaks
    # left to right, largest first; leaf n merges with the peaks of the trailing set bits of n.
    node = hashlib.sha256(b"\x00" + h).digest()
    k = n
    while k & 1:
        node = hashlib.sha256(b"\x01" + peaks.pop() + node).digest()
        k >>= 1
    peaks.append(node)


class R:
    def __init__(self, b):
        self.w = struct.unpack("<%dI" % (len(b) // 4), b); self.i = 0

    def u32(self):
        self.i += 1; return self.w[self.i - 1]

    def i64(self):
        lo, hi = self.u32(), self.u32(); v = lo | (hi << 32); return v - (1 << 64) if v >> 63 else v

    def b32(self):
        return bytes(self.u32() for _ in range(32))


def main():
    cmd = sys.argv[1]
    if cmd == "fetch":
        a, b, out = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
        with open(out, "w") as f:
            for h in range(a, b + 1):
                f.write(f"{h} {cli('getblockheader', cli('getblockhash', h), 'false')}\n")
        print(f"fetched {b - a + 1} headers to {out}")
    elif cmd == "stream":
        hs = read_headers(sys.argv[2]); assert hs[0][0] == 0, "a stream from genesis"
        body = b"".join(h for _, h in hs); assert all(len(h) == 80 for _, h in hs)
        open(sys.argv[3], "wb").write(empty_state() + words(len(hs)) + body)
        print(f"stream {len(hs)} headers, {os.path.getsize(sys.argv[3])} bytes")
    elif cmd == "check":
        r = R(open(sys.argv[2], "rb").read()); hs = read_headers(sys.argv[3])
        digest = r.b32()
        height = r.i64(); hsh = r.b32(); work = r.b32(); bits, time, eft = r.u32(), r.u32(), r.u32()
        recent = [r.u32() for _ in range(r.u32())]; max_time = r.u32()
        peaks = [r.b32() for _ in range(r.u32())]
        assert r.i == len(r.w), "journal has trailing words"
        assert digest == hashlib.sha256(empty_state()).digest(), "input digest is not sha256(empty state)"
        node = json.loads(cli("getblockheader", cli("getblockhash", height)))
        disp = lambda b: b[::-1].hex()
        ok_hash = disp(hsh) == node["hash"]; ok_work = disp(work) == node["chainwork"]
        mine = []
        for i, (h, x) in enumerate(hs):
            mmr_append(mine, h, dsha(x))
        ok_mmr = mine == peaks
        ok_misc = bits == int(node["bits"], 16) and time == node["time"] and height == hs[-1][0]
        print(f"end height {height} hash {disp(hsh)} chainwork {disp(work)}")
        print(f"node: hash {'MATCH' if ok_hash else 'DIFFERENT'}, chainwork {'MATCH' if ok_work else 'DIFFERENT'}, "
              f"bits/time {'MATCH' if ok_misc else 'DIFFERENT'}; MMR peaks ({len(peaks)}) {'MATCH' if ok_mmr else 'DIFFERENT'}")
        sys.exit(0 if ok_hash and ok_work and ok_mmr and ok_misc else 1)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
