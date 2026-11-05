#!/usr/bin/env python3
"""Core's own script and transaction test vectors as cases for the guest (ROADMAP I1; guest mode 10).

Reads a Bitcoin Core source tree (the one the guest is built from) and writes a JSON list of cases that
`host core-vectors` runs through the guest. Every table (opcode names, flag names and bits, script errors) is
parsed from that tree, not retyped, and every check follows Core's own test driver:

  script_tests.json  test/script_tests.cpp DoTest: crediting + spending tx (test/util/transaction_utils.cpp), the
                     exact ScriptError, then 16 flag variants (Core: random; here a fixed seed) checking validity only.
  tx_valid.json      test/transaction_tests.cpp: CheckTransaction + every input under ~excluded; each flag removed
                     (TrimFlags) still valid; each single exclusion dropped (ExcludeIndividualFlags) invalid.
  tx_invalid.json    BADTX: CheckTransaction refuses. Else invalid under the flags; each flag added (FillFlags) still
                     invalid; each single flag removed (ExcludeIndividualFlags) valid.

usage: core_vectors.py <bitcoin-core dir> <out.json> [--no-variants]
"""
import hashlib, json, random, re, sys
from decimal import Decimal

CORE = sys.argv[1]
OUT = sys.argv[2]
VARIANTS = "--no-variants" not in sys.argv[3:]
src = lambda p: open(f"{CORE}/src/{p}").read()

# ---- tables from Core's source ---------------------------------------------------------------------------------
enum = {}
for name, val in re.findall(r"^\s*(OP_\w+)\s*=\s*(0x[0-9a-fA-F]+|\d+|OP_\w+)\s*,", src("script/script.h"), re.M):
    enum[name] = enum[val] if val.startswith("OP_") else int(val, 0)
MAX_OPCODE = enum[re.search(r"MAX_OPCODE\s*=\s*(OP_\w+)", src("script/script.h")).group(1)]
OPNAMES = {}
for label, name in re.findall(r'case\s+(OP_\w+)\s*:\s*return\s+"([^"]+)"', src("script/script.cpp")):
    op = enum[label]
    if op > MAX_OPCODE or (op < enum["OP_NOP"] and op != enum["OP_RESERVED"]) or name == "OP_UNKNOWN":
        continue
    OPNAMES[name] = op
    if name.startswith("OP_"):
        OPNAMES[name[3:]] = op

ibits = {}
for name, expr in re.findall(r"^\s*(SCRIPT_VERIFY_\w+)\s*=\s*([^,\n]+),", src("script/interpreter.h"), re.M):
    m = re.fullmatch(r"\(\s*1U\s*<<\s*(\d+)\s*\)", expr.strip())
    ibits[name] = (1 << int(m.group(1))) if m else (0 if expr.strip() == "0" else None)
FLAGS = {n: ibits[v] for n, v in re.findall(r'\{std::string\("(\w+)"\),\s*\(unsigned int\)(SCRIPT_VERIFY_\w+)\}',
                                            src("test/transaction_tests.cpp"))}
assert len(FLAGS) == 21 and all(v for v in FLAGS.values()), FLAGS
F = lambda n: FLAGS[n]

errs = re.search(r"typedef enum ScriptError_t\s*\{(.*?)\}", src("script/script_error.h"), re.S).group(1)
codes, i = {}, 0
for name, val in re.findall(r"(SCRIPT_ERR_\w+)\s*(?:=\s*(\d+))?\s*,", errs):
    i = int(val) if val else i
    codes[name] = i
    i += 1
ERRS = {s: codes[e] for e, s in re.findall(r'\{(SCRIPT_ERR_\w+),\s*"(\w+)"\}', src("test/script_tests.cpp"))}
assert ERRS["OK"] == 0 and "EVAL_FALSE" in ERRS

# ---- script parsing and serialisation, as Core's ---------------------------------------------------------------
def push_data(d):
    n = len(d)
    if n < 0x4C: return bytes([n]) + d
    if n <= 0xFF: return b"\x4c" + bytes([n]) + d
    if n <= 0xFFFF: return b"\x4d" + n.to_bytes(2, "little") + d
    return b"\x4e" + n.to_bytes(4, "little") + d

def scriptnum(v):  # CScriptNum::serialize
    if v == 0: return b""
    neg, a, out = v < 0, abs(v), bytearray()
    while a:
        out.append(a & 0xFF); a >>= 8
    if out[-1] & 0x80: out.append(0x80 if neg else 0)
    elif neg: out[-1] |= 0x80
    return bytes(out)

def push_int(v):  # CScript::push_int64
    if v == -1 or 1 <= v <= 16: return bytes([v + enum["OP_1"] - 1])
    if v == 0: return b"\x00"
    return push_data(scriptnum(v))

def parse_script(s):  # core_read.cpp ParseScript
    out = bytearray()
    for w in re.split(r"[ \t\n]", s):
        if not w: continue
        if w.isdigit() or (w[0] == "-" and len(w) > 1 and w[1:].isdigit()):
            v = int(w)
            if not -0xFFFFFFFF <= v <= 0xFFFFFFFF: raise ValueError("number range")
            out += push_int(v)
        elif w[:2] == "0x" and len(w) > 2 and len(w) % 2 == 0 and re.fullmatch(r"[0-9a-fA-F]+", w[2:]):
            out += bytes.fromhex(w[2:])
        elif len(w) >= 2 and w[0] == "'" and w[-1] == "'":
            out += push_data(w[1:-1].encode())
        else:
            out.append(OPNAMES[w])
    return bytes(out)

def cs(n):
    if n < 0xFD: return bytes([n])
    if n <= 0xFFFF: return b"\xfd" + n.to_bytes(2, "little")
    if n <= 0xFFFFFFFF: return b"\xfe" + n.to_bytes(4, "little")
    return b"\xff" + n.to_bytes(8, "little")

def ser_tx(version, vin, vout, locktime, witness=True):
    """vin: (txid internal, n, scriptSig, sequence, witness stack); vout: (value, spk)."""
    wit = witness and any(x[4] for x in vin)
    b = version.to_bytes(4, "little", signed=True) + (b"\x00\x01" if wit else b"") + cs(len(vin))
    for t, n, sig, seq, _ in vin: b += t + n.to_bytes(4, "little") + cs(len(sig)) + sig + seq.to_bytes(4, "little")
    b += cs(len(vout))
    for v, spk in vout: b += v.to_bytes(8, "little", signed=True) + cs(len(spk)) + spk
    if wit:
        for *_, st in vin: b += cs(len(st)) + b"".join(cs(len(x)) + x for x in st)
    return b + locktime.to_bytes(4, "little")

def prevouts(outs):  # Core's Vec<CTxOut>
    return cs(len(outs)) + b"".join(v.to_bytes(8, "little", signed=True) + cs(len(spk)) + spk for v, spk in outs)

dsha = lambda b: hashlib.sha256(hashlib.sha256(b).digest()).digest()

def tx_inputs(raw):  # each input's (txid internal, n), from a serialized transaction
    o = 4
    if raw[4] == 0 and raw[5] == 1: o += 2
    def rcs():
        nonlocal o
        f = raw[o]; o += 1
        if f < 0xFD: return f
        k = {0xFD: 2, 0xFE: 4, 0xFF: 8}[f]; v = int.from_bytes(raw[o:o + k], "little"); o += k; return v
    ins = []
    for _ in range(rcs()):
        t, n = raw[o:o + 32], int.from_bytes(raw[o + 32:o + 36], "little"); o += 36
        ln = rcs()          # not `o += rcs() + 4`: Python would read o before rcs() moves it
        o += ln + 4
        ins.append((t, n))
    return ins

# ---- Core's flag helpers (transaction_tests.cpp) ----------------------------------------------------------------
M32 = 0xFFFFFFFF
def trim(f):
    if not f & F("P2SH"): f &= ~F("WITNESS") & M32
    if not f & F("WITNESS"): f &= ~F("CLEANSTACK") & M32
    return f
def fill(f):
    if f & F("CLEANSTACK"): f |= F("WITNESS")
    if f & F("WITNESS"): f |= F("P2SH")
    return f
def exclude_each(flags):
    out = []
    for name in sorted(FLAGS):
        x = trim(flags & ~FLAGS[name] & M32)
        if x != flags and x not in out: out.append(x)
    return out
def parse_flags(s):
    if s in ("", "NONE"): return 0
    f = 0
    for w in s.split(","): f |= FLAGS[w]
    return f

# ---- cases ------------------------------------------------------------------------------------------------------
cases = []
def case(kind, tx, outs, inp, flags, label, valid=None, code=None):
    cases.append({"kind": kind, "tx": tx.hex(), "prevouts": prevouts(outs).hex(), "input": inp, "flags": flags & M32,
                  "label": label, "valid": valid, "code": code})

rng = random.Random(0x11B5)
for idx, t in enumerate(json.load(open(f"{CORE}/src/test/data/script_tests.json"), parse_float=Decimal)):
    pos, wit, value = 0, [], 0
    if t and isinstance(t[0], list):
        wit = [bytes.fromhex(x) for x in t[0][:-1]]
        value = int(Decimal(t[0][-1]) * 100_000_000)
        pos = 1
    if len(t) < 4 + pos: continue
    sig, spk = parse_script(t[pos]), parse_script(t[pos + 1])
    flags, err = parse_flags(t[pos + 2]), ERRS[t[pos + 3]]
    if flags & F("CLEANSTACK"): flags |= F("P2SH") | F("WITNESS")
    credit = ser_tx(1, [(b"\x00" * 32, 0xFFFFFFFF, b"\x00\x00", 0xFFFFFFFF, [])], [(value, spk)], 0, witness=False)
    spend = ser_tx(1, [(dsha(credit), 0, sig, 0xFFFFFFFF, wit)], [(value, b"")], 0)
    label = f"script_tests[{idx}]"
    case(0, spend, [(value, spk)], 0, flags, label, valid=(err == 0), code=(1 if err == 0 else -err - 1))
    if VARIANTS:
        for v in range(16):
            extra = rng.getrandbits(16)
            f = (flags & ~extra) if err == 0 else (flags | extra)
            if f & F("CLEANSTACK") and ~f & (F("P2SH") | F("WITNESS")): continue
            if f & F("WITNESS") and ~f & F("P2SH"): continue
            case(0, spend, [(value, spk)], 0, f, f"{label} flags {f:x}", valid=(err == 0))

for which in ("tx_valid", "tx_invalid"):
    for idx, t in enumerate(json.load(open(f"{CORE}/src/test/data/{which}.json"), parse_float=Decimal)):
        if not isinstance(t[0], list): continue
        assert len(t) == 3
        m = {}
        for i in t[0]:
            m[(bytes.fromhex(i[0])[::-1], int(i[1]) & M32)] = (int(i[3]) if len(i) >= 4 else 0, parse_script(i[2]))
        raw = bytes.fromhex(t[1])
        outs = [m[k] for k in tx_inputs(raw)]
        label = f"{which}[{idx}]"
        if which == "tx_valid":
            excl = parse_flags(t[2])
            assert (~excl & M32) == fill(~excl & M32), label
            case(1, raw, outs, 0, ~excl, label, valid=True)
            if VARIANTS:
                for name in sorted(FLAGS):
                    case(1, raw, outs, 0, trim(~(excl | FLAGS[name]) & M32), f"{label} -{name}", valid=True)
                for f in exclude_each(excl):
                    case(1, raw, outs, 0, ~f, f"{label} maximal {f:x}", valid=False)
        else:
            if t[2] == "BADTX":
                case(1, raw, outs, 0, 0, label, code=-1000)
                continue
            fl = parse_flags(t[2])
            assert fl == fill(fl), label
            case(1, raw, outs, 0, fl, label, valid=False)
            if VARIANTS:
                for name in sorted(FLAGS):
                    case(1, raw, outs, 0, fill(fl | FLAGS[name]), f"{label} +{name}", valid=False)
                for f in exclude_each(fl):
                    case(1, raw, outs, 0, f, f"{label} minimal {f:x}", valid=True)

json.dump(cases, open(OUT, "w"))
kinds = {k: sum(1 for c in cases if c["label"].startswith(k)) for k in ("script_tests", "tx_valid", "tx_invalid")}
print(f"{len(cases)} cases {kinds} -> {OUT}")
