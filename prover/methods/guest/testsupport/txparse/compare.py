#!/usr/bin/env python3
"""Lines up Core's harness output with the Rust parsers' (test-txparse.sh) and applies the rules below.

  compare.py corpus.txt core.out rust.out [--expect-mismatch]

A MISMATCH is a case where a Rust parser on the soundness path reads bytes Core accepts differently from Core, or
accepts bytes Core refuses:
  tx_io (core guest: trie keys, output script hashes)  Core (with witness) accepts => tx_io accepts, the same outpoints,
      the same output scripts. Core refuses => nothing to compare: every guest entry point parses the same bytes with
      Core first and traps (no proof), so tx_io's answer is unused (counted, not a mismatch).
  prevout_fields (core guest: undo records, spent scripts)  as tx_io, against Core's std::vector<CTxOut> reader.
  the guest's raw-byte txid / wtxid (HAZYNC_RAW_TXID)  when Core consumed every byte, equal to Core's own hashes.
  facts::parse_tx (verifiers' transaction facts)  accepts only what Core (with witness) accepts, then the same
      length, txid, outpoints, outputs. It refuses some Core-readable forms by design (no inputs; > 2^20 inputs or
      outputs): counted as "stricter", not a mismatch.
  hints / utreexo parse_tx (zAPP guests, stripped form)  as facts, against Core without witness.
--expect-mismatch: the positive control (a lenient compact-size reader); exit 0 only if it IS caught.
"""
import sys
from collections import Counter


def main():
    corpus, core_f, rust_f = sys.argv[1:4]
    expect = "--expect-mismatch" in sys.argv
    core = open(core_f).read().split("\n")
    rust = open(rust_f).read().split("\n")
    ci = ri = 0
    stats, bad = Counter(), []

    def mism(kind, data, c, r):
        bad.append((kind, data, c, r))

    for line in open(corpus):
        parts = line.split()
        k, h = parts[0], (parts[1] if len(parts) > 1 else "")
        if k == "T":
            cw, cn = core[ci], core[ci + 1]; ci += 2
            io, facts, hints, utx = rust[ri:ri + 4]; ri += 4
            cwf, cnf = cw.split(), cn.split()
            w_ok, n_ok = cwf[2] == "ok", cnf[2] == "ok"
            stats["T"] += 1; stats["T core-witness ok" if w_ok else "T core-witness refused"] += 1
            # tx_io
            iof = io.split()
            if w_ok:
                # cw: T W ok used txid wtxid stxid swtxid haswit nin ins nout outs
                _, _, _, used, txid, wtxid, stxid, swtxid, hw, nin, ins, nout, outs = cwf
                core_scripts = "." if outs == "." else ",".join(o.split(":", 1)[1] for o in outs.split(","))
                if iof[2] != "ok" or iof[3] != nin or iof[4] != ins or iof[5] != nout or (len(iof) > 6 and iof[6] != core_scripts):
                    mism("tx_io", h, cw, io)
                if stxid != "-" and (stxid != txid or swtxid != wtxid):
                    mism("raw-txid", h, cw, "")
                if stxid != "-": stats["raw txid checked"] += 1
                # facts
                ff = facts.split()
                if ff[2] == "ok":
                    if ff[3:] != [used, txid, nin, ins, nout, outs]:
                        mism("facts", h, cw, facts)
                else:
                    if nin == "0" or int(nin) > 1 << 20 or int(nout) > 1 << 20:
                        stats["facts stricter (by design)"] += 1
                    else:
                        mism("facts-refuses-core-valid", h, cw, facts)
            else:
                if iof[2] == "ok": stats["tx_io reads what Core refuses (unused: the guest traps first)"] += 1
                if facts.split()[2] == "ok": mism("facts-accepts-core-invalid", h, cw, facts)
            # hints / utreexo against Core without witness
            for name, rl in (("hints", hints), ("utreexo", utx)):
                rf = rl.split()
                if n_ok:
                    _, _, _, used, txid, _, _, _, _, nin, ins, nout, outs = cnf
                    if rf[2] == "ok":
                        if rf[3:] != [used, txid, nin, ins, nout, outs]:
                            mism(name, h, cn, rl)
                    elif nin == "0" or int(nin) > 1 << 20 or int(nout) > 1 << 20:
                        stats[f"{name} stricter (by design)"] += 1
                    else:
                        mism(f"{name}-refuses-core-valid", h, cn, rl)
                elif rf[2] == "ok":
                    mism(f"{name}-accepts-core-invalid", h, cn, rl)
        else:
            c = core[ci]; ci += 1
            r = rust[ri]; ri += 1
            stats["P"] += 1
            cf, rf = c.split(), r.split()
            if cf[1] == "ok":
                stats["P core ok"] += 1
                if rf[1] != "ok" or rf[2:] != cf[2:]:
                    mism("prevout_fields", h, c, r)
            elif rf[1] == "ok":
                stats["prevout_fields reads what Core refuses (unused: the guest traps first)"] += 1
    for k in sorted(stats):
        print(f"  {k}: {stats[k]}")
    by = Counter(b[0] for b in bad)
    print(f"  MISMATCHES: {len(bad)} {dict(by)}")
    seen = Counter()
    for kind, data, c, r in bad:
        seen[kind] += 1
        if seen[kind] <= 3:
            print(f"    [{kind}] input {data[:400]}{'...' if len(data) > 400 else ''}\n      core: {c[:300]}\n      rust: {r[:300]}")
    if expect:
        sys.exit(0 if bad else 1)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
