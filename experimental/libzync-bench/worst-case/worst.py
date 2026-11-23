#!/usr/bin/env python3
"""worst.py <regtest host> <dir> <stage> [case ...]: worst-case VALID blocks for the libzync prover, built on regtest
with Bitcoin Core 28.0, accepted by Core (submitblock), then executed (dev mode, nothing proved) by the regtest guest
with the same host command a prover runs (`libzync-prove-range`), every session's cycles recorded.

Stages:
  guest  build the base chain + preparation blocks, the guest anchor at its tip, then every guest case (CASES below):
         Core's verdict and time, the guest's verdict, per-session LZ_STATS, peak RSS.
  full   (its own <dir> and HZ_PORT) Core only: a chain with enough heavy coins to fill a whole 1 MB block with heavy
         inputs, then the full-size worst block (SIGHASH_NONE and SIGHASH_ALL variants): Core's verdict, and its
         validation time with the default script threads and with -par=1 (node restarted, so no warm signature or
         script cache; the block is re-validated by reconsiderblock). The guest does not run it: its per-input cost is
         measured on the sample blocks (RESULT.md).

Construction (RESULT.md explains why these are the worst cases for this prover):
  HEAVY coin: bare scriptPubKey S, 6,644 bytes, 201 non-push opcodes:
      9 x [OP_0 <sig> OP_1 <20 distinct valid keys> 20 OP_CHECKMULTISIG] + [OP_0 <sig> OP_1 <11 keys> 11 OP_CHECKMULTISIG] + OP_1
    <sig> = strict-DER r=1, s=1 + one sighash-type byte (DERSIG passes; it is never a valid signature).
    Each CHECKMULTISIG(m=1) tries the signature against every key: 9*20 + 11 = 191 legacy sighashes and 191 failing
    ECDSA verifications per input. The script leaves 10 falses and a true: valid (CLEANSTACK, NULLFAIL, LOW_S are
    policy, not consensus). Its 200 legacy sigops are counted in the block that CREATES it (bare outputs), never in the spending
    block: the spending input has an EMPTY scriptSig, 41 bytes.
  MEM coin: bare scriptPubKey M, 9,818 bytes: 18 pushes of 520 bytes, OP_0, 201 x (OP_0 OP_CHECKSIG), OP_1. Every
    CHECKSIG has an empty signature and an empty key: no hashing, no EC work, but Core copies the scriptCode (9.8 kB) and
    FindAndDelete builds a new one for EACH call. The guest's allocator never frees (bump), so heap grows ~201 x that
    per input. 201 legacy sigops, counted in the creating block (95 per prep block).
  LIGHT coin: bare OP_TRUE, spent with an empty scriptSig (41-byte input, no signature work).
Env: BITCOIND (Core 28.0 bitcoind), HZ_CORE_SRC (a Core tree with test/functional), HZ_TIMED (timed.py), HZ_CHUNKS
     (space-separated chunk counts to run each guest case at, default "2"), HZ_ONLY (comma-separated case names).
"""
import json, os, re, shutil, subprocess, sys, time

CORE_SRC = os.environ.get("HZ_CORE_SRC", "/root/worst-case/core/src")
sys.path.insert(0, CORE_SRC + "/test/functional")
from test_framework.blocktools import add_witness_commitment, create_block, create_coinbase  # noqa: E402
from test_framework.messages import COutPoint, CTransaction, CTxIn, CTxOut, hash256, ser_uint256  # noqa: E402
from test_framework.key import ECKey  # noqa: E402
from test_framework.script import CScript, OP_0, OP_1, OP_11, OP_CHECKMULTISIG, OP_CHECKSIG, OP_RETURN, OP_TRUE  # noqa: E402

BIN = os.environ.get("BITCOIND", "/root/worst-case/core/bitcoin-28.0/bin/bitcoind")
CLI = os.path.join(os.path.dirname(BIN), "bitcoin-cli")
HERE = os.path.dirname(os.path.abspath(__file__))
TIMED = os.environ.get("HZ_TIMED", HERE + "/timed.py")
HOST, ROOT, STAGE = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2]), sys.argv[3]
ONLY = set(os.environ["HZ_ONLY"].split(",")) if os.environ.get("HZ_ONLY") else None
DATA, BLOCKS, OUT = ROOT + "/node", ROOT + "/blocks", ROOT + "/out"
ANYONE = CScript([OP_TRUE])
PORT = int(os.environ.get("HZ_PORT", "18890"))
RPC = ["-regtest", f"-datadir={DATA}", f"-rpcport={PORT}"]
MAX_STRIPPED = 1_000_000          # 4 MWU, no witness anywhere: 1,000,000 bytes of block
SIG_NONE, SIG_ALL = 0x02, 0x01
MEM_PER_PREP = 95                 # 95 * 201 = 19,095 legacy sigops; 95 * 9.8 kB < 1 MB
HEAVY_PER_PREP = 99               # 99 * 200 legacy sigops (10 CHECKMULTISIGs, each counted 20 in an output) = 19,800 <= 20,000
N_LIGHT = 24_600                  # light coins: enough to fill a block with 41-byte inputs


def cli(*a):
    r = subprocess.run([CLI] + RPC + ["-stdin", str(a[0])], input="".join(f"{x}\n" for x in a[1:]), capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr)
    return r.stdout.strip()


def rev_hex(n):
    return ser_uint256(n)[::-1].hex()


class Chain:   # as core_rules/rig.py's Chain (only what these blocks need: no BIP30 staging)
    def __init__(self):
        self.times, self.gen_time = [], 1296688602
        self.tip, self.height = int(cli("getbestblockhash"), 16), 0
        self.utxo = {}   # (txid, n) -> (value, spk, height, flags, creation mtp)

    def mtp(self, h):
        ts = ([self.gen_time] + self.times)[max(0, h - 10):h + 1]
        return sorted(ts)[len(ts) // 2]

    def recent(self):
        return list(reversed(([self.gen_time] + self.times)[-11:]))

    def block(self, txs=(), fees=0):
        h = self.height + 1
        b = create_block(self.tip, create_coinbase(h, script_pubkey=ANYONE, fees=fees), ([self.gen_time] + self.times)[-1] + 1,
                         version=4, txlist=list(txs))
        add_witness_commitment(b)
        b.solve()
        return b

    def accept(self, b):
        h = self.height + 1
        for i, t in enumerate(b.vtx):
            t.rehash()
            for n, o in enumerate(t.vout):
                if bytes(o.scriptPubKey)[:1] != bytes([OP_RETURN]):
                    self.utxo[(t.sha256, n)] = (o.nValue, bytes(o.scriptPubKey), h, 1 if i == 0 else 0, self.mtp(h - 1))
            if i:
                for x in t.vin:
                    self.utxo.pop((x.prevout.hash, x.prevout.n), None)
        self.times.append(b.nTime)
        self.tip, self.height = b.sha256, h


def to_json(c, b, path):
    h = c.height + 1
    made = {}
    for t in b.vtx[1:]:
        t.rehash()
        for n, o in enumerate(t.vout):
            made[(t.sha256, n)] = (o.nValue, bytes(o.scriptPubKey), h, 0, c.mtp(h - 1))
    txs = []
    for t in b.vtx[1:]:
        pv = []
        for x in t.vin:
            u = c.utxo.get((x.prevout.hash, x.prevout.n)) or made[(x.prevout.hash, x.prevout.n)]
            pv.append({"value": u[0], "spk": u[1].hex(), "coin_height": u[2], "coin_is_coinbase": int(u[3] & 1), "coin_mtp": u[4]})
        txs.append({"raw": t.serialize().hex(), "prevouts": pv})
    json.dump({"height": h, "bits": b.nBits, "time": b.nTime, "nonce": b.nNonce, "version": b.nVersion,
               "prev": rev_hex(b.hashPrevBlock), "merkle": rev_hex(b.hashMerkleRoot),
               "coinbase_hex": b.vtx[0].serialize().hex(), "coinbase_flags": 1, "txs": txs, "recent_times": c.recent()}, open(path, "w"))


def spend(coins, outs, scriptsigs=None):
    t = CTransaction()
    t.version = 2
    t.vin = [CTxIn(COutPoint(a, b), (scriptsigs or {}).get(i, b""), 0xffffffff) for i, (a, b) in enumerate(coins)]
    t.vout = [CTxOut(v, s) for v, s in outs]
    t.rehash()
    return t


# ---- the heavy script --------------------------------------------------------------------------------------------
def sig(ht):
    return bytes([0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01, ht])   # strict DER, r = s = 1: parses, never verifies


def keys(n):
    out = []
    for _ in range(n):
        k = ECKey(); k.generate(); out.append(k.get_pubkey().get_bytes())   # valid points: libsecp runs the full verify
    return out


def heavy_script(ht, ks):
    """191 failing ECDSA checks, each over a legacy sighash; exactly 201 non-push opcodes."""
    it = iter(ks)
    parts = []
    for n in [20] * 9 + [11]:
        parts += [OP_0, sig(ht), OP_1] + [next(it) for _ in range(n)] + [n if n > 16 else OP_11, OP_CHECKMULTISIG]
    parts.append(OP_1)
    s = CScript(parts)
    assert len(bytes(s)) <= 10_000, len(bytes(s))
    return s


def mem_script():
    s = CScript([bytes([i]) * 520 for i in range(1, 19)] + [OP_0] + [OP_0, OP_CHECKSIG] * 201 + [OP_1])
    assert len(bytes(s)) <= 10_000, len(bytes(s))
    return s


# ---- the base chain ----------------------------------------------------------------------------------------------
STATE = ROOT + "/chain.json"


def mature(c):
    h = c.height + 1
    r = sorted([o for o, u in c.utxo.items() if u[3] & 1 and u[2] + 100 <= h and u[1] == bytes(ANYONE)], key=lambda o: c.utxo[o][2])
    return r[0]


def submit(c, b, path_json=None, keep=True):
    if path_json:
        to_json(c, b, path_json)
    hx = b.serialize().hex()
    t0 = time.time()
    got = cli("submitblock", hx)
    dt = time.time() - t0
    return got, dt


def base(c, n_heavy_sets, n_mem_sets):
    """110 plain blocks; one block of N_LIGHT light coins; heavy coins (both sighash types) in prep blocks."""
    prep = {"light": [], "heavy": {SIG_NONE: [], SIG_ALL: []}, "mem": []}
    for _ in range(110):
        b = c.block(); got, _ = submit(c, b, f"{BLOCKS}/main/block_{c.height + 1}.json"); assert got in ("", "null"), got; c.accept(b)
    # light coins
    x = mature(c); v = c.utxo[x][0]; each = (v - 0) // N_LIGHT
    t = spend([x], [(each, bytes(ANYONE))] * N_LIGHT)
    b = c.block([t]); got, _ = submit(c, b, f"{BLOCKS}/main/block_{c.height + 1}.json"); assert got in ("", "null"), got; c.accept(b)
    prep["light"] = [(t.sha256, i) for i in range(N_LIGHT)]
    # heavy coins: each coin its own 191 distinct keys
    for ht, nsets in n_heavy_sets.items():
        for _ in range(nsets):
            x = mature(c); v = c.utxo[x][0]; each = v // HEAVY_PER_PREP
            outs = [(each, bytes(heavy_script(ht, keys(191)))) for _ in range(HEAVY_PER_PREP)]
            t = spend([x], outs)
            b = c.block([t]); got, _ = submit(c, b, f"{BLOCKS}/main/block_{c.height + 1}.json"); assert got in ("", "null"), got; c.accept(b)
            prep["heavy"][ht] += [(t.sha256, i) for i in range(HEAVY_PER_PREP)]
    for _ in range(n_mem_sets):
        x = mature(c); v = c.utxo[x][0]; each = v // MEM_PER_PREP
        t = spend([x], [(each, bytes(mem_script()))] * MEM_PER_PREP)
        b = c.block([t]); got, _ = submit(c, b, f"{BLOCKS}/main/block_{c.height + 1}.json"); assert got in ("", "null"), got; c.accept(b)
        prep["mem"] += [(t.sha256, i) for i in range(MEM_PER_PREP)]
    return prep


# ---- the cases ---------------------------------------------------------------------------------------------------
def fill_tx(c, first, scriptsigs=None, light=None):
    """One transaction: `first` inputs, then light inputs until the block is ~1,000,000 bytes; one OP_TRUE output."""
    light = list(light)
    coins = list(first)
    t = spend(coins, [(1, bytes(ANYONE))], scriptsigs)
    room = MAX_STRIPPED - 80 - 3 - 400 - len(t.serialize())     # header, tx count, coinbase (generous), this tx so far
    add = room // 41 - 3                                         # 41 bytes per light input (+ the vin compactsize growth)
    coins += light[:add]
    t = spend(coins, [(sum(c.utxo[o][0] for o in coins), bytes(ANYONE))], scriptsigs)
    return t


def case_block(c, name, prep):
    L = prep["light"]
    if name == "light_only":                     # 24k inputs, no signatures: the most inputs a block can spend
        return c.block([fill_tx(c, [], light=L)])
    m = re.fullmatch(r"(none|all)_k(\d+)", name)  # k heavy inputs first, then light ones, one ~1 MB transaction
    if m:
        ht = SIG_NONE if m.group(1) == "none" else SIG_ALL
        k = int(m.group(2))
        return c.block([fill_tx(c, prep["heavy"][ht][:k], light=L)])
    m = re.fullmatch(r"mem_k(\d+)", name)        # k memory-heavy inputs first, then light ones
    if m:
        return c.block([fill_tx(c, prep["mem"][:int(m.group(1))], light=L)])
    if name == "unsplit":                         # ONE input doing 2 x 191 checks: its scriptSig is a heavy script too
        ss = bytes(heavy_script(SIG_ALL, keys(191)))[:-1]   # the scriptSig without its final OP_1 (left: 10 falses)
        return c.block([fill_tx(c, prep["heavy"][SIG_ALL][-1:], scriptsigs={0: ss}, light=L)])
    if name == "max_outputs":                     # the most coins a block can create: empty-script outputs, 9 bytes each
        x = mature(c); v = c.utxo[x][0]
        n = (MAX_STRIPPED - 80 - 400 - 60) // 9 - 2
        return c.block([spend([x], [(v // n, b"")] * n)])
    raise KeyError(name)


CASES = ["light_only", "max_outputs", "none_k8", "none_k24", "all_k8", "all_k24", "unsplit", "mem_k1140"]


def run_guest(c, name, b, nch):
    h = c.height + 1
    d = f"{BLOCKS}/{name}"
    o = f"{OUT}/{name}_c{nch}"
    shutil.rmtree(o, ignore_errors=True)
    env = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_RIG="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS=str(nch), HAZYNC_SEG_PO2="21")
    log = f"{OUT}/{name}_c{nch}.log"
    if os.path.exists(log):
        os.remove(log)
    r = subprocess.run(["python3", "-I", TIMED, log, "--", HOST, "libzync-prove-range", str(h), str(h), d, f"{BLOCKS}/anchor.json", o], env=env)
    t = open(log, errors="replace").read()
    stats = re.findall(r"LZ_STATS (\w+) h=\d+ segments=(\d+) total_cycles=(\d+) user_cycles=(\d+)", t)
    timed = re.findall(r"TIMED .*", t)
    return r.returncode, stats, timed[-1] if timed else "", t


def main():
    os.makedirs(BLOCKS + "/main", exist_ok=True); os.makedirs(OUT, exist_ok=True)
    if not os.path.exists(STATE):
        shutil.rmtree(DATA, ignore_errors=True)
    os.makedirs(DATA, exist_ok=True)
    p = start_node()
    try:
        print("core:", json.loads(cli("getnetworkinfo"))["subversion"], "par: default (nproc", os.cpu_count(), ")", flush=True)
        if STAGE == "guest":
            guest_stage()
        elif STAGE == "full":
            p = full_stage(p)
    finally:
        cli("stop"); p.wait()


def start_node(extra=()):
    p = subprocess.Popen([BIN] + RPC + [f"-port={PORT + 1}", "-listen=0", "-txindex=0", "-testactivationheight=bip34@110", "-debug=bench",
                          "-maxmempool=5", "-dbcache=2000"] + list(extra), stdout=subprocess.DEVNULL)
    for _ in range(600):
        try:
            cli("getblockcount"); return p
        except RuntimeError:
            time.sleep(0.3)
    raise RuntimeError("bitcoind did not start")


def save(c, prep):
    import pickle
    pickle.dump((c.__dict__, prep), open(STATE + ".pkl", "wb")); open(STATE, "w").write("ok")


def load():
    import pickle
    d, prep = pickle.load(open(STATE + ".pkl", "rb"))
    c = Chain.__new__(Chain); c.__dict__.update(d)
    assert rev_hex(c.tip) == cli("getbestblockhash"), "node tip != saved chain"
    return c, prep


def guest_stage():
    if not os.path.exists(STATE):
        c = Chain()
        t0 = time.time()
        prep = base(c, {SIG_NONE: 1, SIG_ALL: 1}, 12)
        print(f"base chain to {c.height} in {time.time() - t0:.0f} s", flush=True)
        save(c, prep)
    c, prep = load()
    anchor = f"{BLOCKS}/anchor.json"
    if not os.path.exists(anchor):
        o = f"{OUT}/base"
        env = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_RIG="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS="1", HAZYNC_SEG_PO2="21")
        r = subprocess.run(["python3", "-I", TIMED, f"{OUT}/base.log", "--", HOST, "libzync-prove-range", "1", str(c.height),
                            BLOCKS + "/main", "genesis", o], env=env)
        assert r.returncode == 0, open(f"{OUT}/base.log", errors="replace").read()[-1500:]
        shutil.copy(f"{o}/state_{c.height}.json", anchor)
        for ext in ("coinlog", "trie"):
            if os.path.exists(f"{o}/state_{c.height}.{ext}"):
                shutil.copy(f"{o}/state_{c.height}.{ext}", f"{BLOCKS}/anchor.{ext}")
        print("anchor at", c.height, flush=True)
    for name in CASES:
        if ONLY and name not in ONLY:
            continue
        b = case_block(c, name, prep)
        d = f"{BLOCKS}/{name}"
        shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
        for h in range(1, c.height + 1):
            os.symlink(f"{BLOCKS}/main/block_{h}.json", f"{d}/block_{h}.json")
        to_json(c, b, f"{d}/block_{c.height + 1}.json")
        raw = b.serialize()
        ins = sum(len(t.vin) for t in b.vtx[1:])
        outs = sum(len(t.vout) for t in b.vtx)
        got, dt = submit(c, b)
        bh = rev_hex(int.from_bytes(hash256(raw[:80]), "little"))
        info = json.loads(cli("getblock", bh, "1")) if got in ("", "null") else {}
        core = "accept" if got in ("", "null") else got
        print(f"CASE {name} height={c.height + 1} bytes={len(raw)} weight={info.get('weight')} txs={len(b.vtx)} inputs={ins} outputs={outs} "
              f"core={core} core_submit_s={dt:.2f}", flush=True)
        if core == "accept":
            cli("invalidateblock", bh)
        for nch in [int(x) for x in os.environ.get("HZ_CHUNKS", "2").split()]:
            rc, stats, timed, t = run_guest(c, name, b, nch)
            verdict = "accept" if rc == 0 else "reject/abort"
            why = ""
            if rc:
                bad = [l for l in t.splitlines() if "panicked" in l or "INVALID" in l or "core-reject" in l or "rror" in l]
                why = " | ".join(bad[-3:])[:400]
            print(f"GUEST {name} chunks={nch} verdict={verdict} rc={rc} {timed} {why}", flush=True)
            for s in stats:
                print(f"  LZ_STATS {s[0]} segments={s[1]} total_cycles={s[2]} user_cycles={s[3]}", flush=True)


def full_stage(p):
    """Core only: the full-size worst block, every input heavy (both sighash types)."""
    if not os.path.exists(STATE):
        c = Chain()
        prep = {"light": [], "heavy": {SIG_NONE: [], SIG_ALL: []}, "mem": []}
        for _ in range(110):
            b = c.block(); got, _ = submit(c, b); assert got in ("", "null"), got; c.accept(b)
        save(c, prep)
    c, prep = load()
    n_full = (MAX_STRIPPED - 80 - 3 - 400 - 60) // 41
    for ht in (SIG_NONE, SIG_ALL):
        need = n_full - len(prep["heavy"][ht])
        spk = bytes(heavy_script(ht, keys(191)))  # one 191-key set for all of these coins (Core keeps no key memo)
        t0 = time.time()
        while need > 0:
            x = mature(c); v = c.utxo[x][0]; each = v // HEAVY_PER_PREP
            t = spend([x], [(each, spk)] * HEAVY_PER_PREP)
            b = c.block([t]); got, _ = submit(c, b); assert got in ("", "null"), got; c.accept(b)
            prep["heavy"][ht] += [(t.sha256, i) for i in range(HEAVY_PER_PREP)]
            need -= HEAVY_PER_PREP
        print(f"full: {len(prep['heavy'][ht])} heavy coins of type {ht:#x} at height {c.height} ({time.time() - t0:.0f} s)", flush=True)
        save(c, prep)
    for ht, nm in ((SIG_NONE, "full_none"), (SIG_ALL, "full_all")):
        coins = prep["heavy"][ht][:n_full]
        t = spend(coins, [(sum(c.utxo[o][0] for o in coins), bytes(ANYONE))])
        while len(t.serialize()) > MAX_STRIPPED - 80 - 3 - 400:
            coins = coins[:-10]; t = spend(coins, [(sum(c.utxo[o][0] for o in coins), bytes(ANYONE))])
        b = c.block([t]); raw = b.serialize()
        open(f"{OUT}/{nm}.hex", "w").write(raw.hex())
        bh = rev_hex(int.from_bytes(hash256(raw[:80]), "little"))
        print(f"CASE {nm} height={c.height + 1} bytes={len(raw)} inputs={len(t.vin)} checks={191 * len(t.vin)} "
              f"sighash_bytes_per_check~{len(t.serialize())} load1={os.getloadavg()[0]:.1f}", flush=True)
        got, dt = submit(c, b)
        core = "accept" if got in ("", "null") else got
        info = json.loads(cli("getblock", bh, "1")) if core == "accept" else {}
        print(f"CORE {nm} par=default core={core} weight={info.get('weight')} submitblock_s={dt:.1f} load1_end={os.getloadavg()[0]:.1f}", flush=True)
        if core != "accept":
            continue
        cli("invalidateblock", bh)
        # once more single-threaded, from a fresh process (empty signature/script caches)
        cli("stop"); p.wait()
        p = start_node(["-par=1"])
        t0 = time.time(); cli("reconsiderblock", bh); dt1 = time.time() - t0
        ok = cli("getbestblockhash") == bh
        print(f"CORE {nm} par=1 connected={ok} reconsiderblock_s={dt1:.1f} load1_end={os.getloadavg()[0]:.1f}", flush=True)
        cli("invalidateblock", bh)
        cli("stop"); p.wait()
        p = start_node()
    return p


if __name__ == "__main__":
    main()
