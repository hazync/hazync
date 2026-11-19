#!/usr/bin/env python3
"""gen_chain.py <out dir>: build the zAPP regtest chain (docs/libzync/ZAPP_TESTING.md §3.2) and write it as raw blocks.

Every block is built with Bitcoin Core's own test_framework and submitted to a throwaway Core regtest node, which must
accept it (`submitblock` returns null). Nothing here is our libzync code: the node is the judge of every block.

Deterministic: fixed keys (sha256 of a name), RFC 6979 ECDSA, BIP340 Schnorr with aux_rand = 32 zero bytes, fixed
block times, fixed transaction layout. Two runs write byte-identical files (checked by regen.sh).

Writes:
  <out>/raw.txt          one line per block: <branch> <height> <hash> <block hex>; branch main = 1..110, a = 111..112,
                         b = 111..113 (b has more work than a: the reorg)
  <out>/scenarios.json   the keys (secret, pubkeys, scripts, regtest addresses) and every named scenario with its
                         txids, outpoints and heights

Env: BITCOIND (Core 28.0, as the CI rig), HZ_CORE_SRC (a Core v28.0 tree, for test/functional/test_framework),
     RT_PORT (RPC port, default 18994; P2P port is RT_PORT+1).
The CI path does not need this script or test_framework: replay.sh submits raw.txt to a plain bitcoind.
"""
import hashlib, json, os, shutil, subprocess, sys, tempfile, time

CORE_SRC = os.environ.get("HZ_CORE_SRC", os.path.expanduser("~/hzb-r7/bitcoin-core"))
sys.path.insert(0, CORE_SRC + "/test/functional")
from test_framework.address import key_to_p2pkh, key_to_p2sh_p2wpkh, key_to_p2wpkh, output_key_to_p2tr  # noqa: E402
from test_framework.blocktools import add_witness_commitment, create_block, create_coinbase  # noqa: E402
from test_framework.key import ECKey, compute_xonly_pubkey, sign_schnorr, tweak_add_privkey  # noqa: E402
from test_framework.messages import COIN, COutPoint, CTransaction, CTxIn, CTxInWitness, CTxOut  # noqa: E402
from test_framework.script import (CScript, LegacySignatureHash, OP_0, OP_1, OP_CHECKSIG, OP_DUP, OP_EQUAL,  # noqa: E402
                                   OP_EQUALVERIFY, OP_HASH160, OP_RETURN, OP_TRUE, SIGHASH_ALL, SIGHASH_DEFAULT,
                                   SegwitV0SignatureHash, TaprootSignatureHash, hash160, taproot_construct)

BIN = os.environ.get("BITCOIND", os.path.expanduser("~/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind"))
CLI = os.path.join(os.path.dirname(BIN), "bitcoin-cli")
PORT = int(os.environ.get("RT_PORT", "18994"))
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
GEN_TIME = 1296688602                    # regtest genesis nTime
TYPES = ["p2pk_c", "p2pk_u", "p2pkh", "p2wpkh", "p2sh_p2wpkh", "p2tr"]
ANYONE = bytes(CScript([OP_TRUE]))
FEE = 1000                               # sats per transaction, unless a scenario says otherwise
DATA = None


def cli(*a):
    r = subprocess.run([CLI, "-regtest", f"-datadir={DATA}", f"-rpcport={PORT}", "-stdin", str(a[0])],
                       input="".join(f"{x}\n" for x in a[1:]), capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr)
    return r.stdout.strip()


# ---- keys and scripts ---------------------------------------------------------------------------------------------
class Key:
    """A harness key: secret = sha256("hazync zapp-regtest key " + name) mod n. Published on purpose (regtest only)."""

    def __init__(self, name):
        self.name = name
        s = int.from_bytes(hashlib.sha256(b"hazync zapp-regtest key " + name.encode()).digest(), "big") % N
        self.secret = s.to_bytes(32, "big")
        self.c = ECKey(); self.c.set(self.secret, True)
        self.u = ECKey(); self.u.set(self.secret, False)
        self.pub_c = self.c.get_pubkey().get_bytes()
        self.pub_u = self.u.get_pubkey().get_bytes()
        self.xonly, _ = compute_xonly_pubkey(self.secret)
        self.tr = taproot_construct(self.xonly)     # BIP86: key path only, no script tree

    def spk(self, t):
        return {
            "p2pk_c": lambda: bytes(CScript([self.pub_c, OP_CHECKSIG])),
            "p2pk_u": lambda: bytes(CScript([self.pub_u, OP_CHECKSIG])),
            "p2pkh": lambda: bytes(CScript([OP_DUP, OP_HASH160, hash160(self.pub_c), OP_EQUALVERIFY, OP_CHECKSIG])),
            "p2wpkh": lambda: bytes(CScript([OP_0, hash160(self.pub_c)])),
            "p2sh_p2wpkh": lambda: bytes(CScript([OP_HASH160, hash160(self.redeem()), OP_EQUAL])),
            "p2tr": lambda: bytes(self.tr.scriptPubKey),
        }[t]()

    def redeem(self):
        return bytes(CScript([OP_0, hash160(self.pub_c)]))

    def address(self, t):
        return {"p2pkh": lambda: key_to_p2pkh(self.pub_c), "p2wpkh": lambda: key_to_p2wpkh(self.pub_c),
                "p2sh_p2wpkh": lambda: key_to_p2sh_p2wpkh(self.pub_c),
                "p2tr": lambda: output_key_to_p2tr(self.tr.output_pubkey)}.get(t, lambda: None)()

    def describe(self, types):
        return {"secret": self.secret.hex(), "pub_compressed": self.pub_c.hex(), "pub_uncompressed": self.pub_u.hex(),
                "xonly_internal": self.xonly.hex(), "taproot_output_key": self.tr.output_pubkey.hex(),
                "scripts": {t: {"spk": self.spk(t).hex(), "address": self.address(t)} for t in types}}


MINER = {t: Key("miner " + t) for t in TYPES}    # coinbases 1..101, one script type each in turn
ALICE = {t: Key("alice " + t) for t in TYPES}    # statement balances: coins under keys we hold, every type
DEPOSIT = Key("deposit")                         # the deposit address (P2WPKH)
CHANGE = Key("change")                           # change and plain transfers (P2WPKH)


def owner(spk):
    """(key, type) of a harness script, or (None, 'anyone') for OP_TRUE."""
    if spk == ANYONE:
        return None, "anyone"
    for k in list(MINER.values()) + list(ALICE.values()) + [DEPOSIT, CHANGE]:
        for t in TYPES:
            if k.spk(t) == spk:
                return k, t
    raise ValueError("no harness key for script " + spk.hex())


# ---- the chain ----------------------------------------------------------------------------------------------------
class Chain:
    def __init__(self):
        self.coins = {}        # (txid int, n) -> (value, spk bytes)
        self.blocks = []       # (branch, height, block)
        self.tip = int(cli("getbestblockhash"), 16)
        self.height = 0
        self.sc = {}           # scenario name -> data (written to scenarios.json)
        self.fee = {}          # txid int -> fee, for the coinbase's claim

    def coin(self, txid, n):
        return self.coins[(txid, n)]

    def tx(self, ins, outs, version=2, locktime=0, seq=0xffffffff, fee=None):
        """ins: [(txid, n)] (coins known to the chain or created earlier in the same block); outs: [(value, spk)],
        a value None takes the rest (inputs - fee). Signs every input for its harness key."""
        prev = [self.coin(*o) for o in ins]
        fee = FEE if fee is None else fee
        rest = sum(v for v, _ in prev) - sum(v for v, _ in outs if v is not None) - fee
        assert rest >= 0, "outputs exceed inputs"
        t = CTransaction()
        t.version, t.nLockTime = version, locktime
        t.vin = [CTxIn(COutPoint(a, b), b"", seq) for a, b in ins]
        t.vout = [CTxOut(rest if v is None else v, s) for v, s in outs]
        spent = [CTxOut(v, s) for v, s in prev]
        wits = []
        for i, (v, spk) in enumerate(prev):
            k, typ = owner(spk)
            stack = []
            if typ in ("p2pk_c", "p2pk_u", "p2pkh"):
                h, _ = LegacySignatureHash(CScript(spk), t, i, SIGHASH_ALL)
                sig = (k.u if typ == "p2pk_u" else k.c).sign_ecdsa(h, rfc6979=True) + bytes([SIGHASH_ALL])
                t.vin[i].scriptSig = CScript([sig]) if typ != "p2pkh" else CScript([sig, k.pub_c])
            elif typ in ("p2wpkh", "p2sh_p2wpkh"):
                code = CScript([OP_DUP, OP_HASH160, hash160(k.pub_c), OP_EQUALVERIFY, OP_CHECKSIG])
                sig = k.c.sign_ecdsa(SegwitV0SignatureHash(code, t, i, SIGHASH_ALL, v), rfc6979=True) + bytes([SIGHASH_ALL])
                stack = [sig, k.pub_c]
                if typ == "p2sh_p2wpkh":
                    t.vin[i].scriptSig = CScript([k.redeem()])
            elif typ == "p2tr":
                m = TaprootSignatureHash(t, spent, SIGHASH_DEFAULT, i)
                stack = [sign_schnorr(tweak_add_privkey(k.secret, k.tr.tweak), m)]   # aux_rand = 32 zero bytes
            w = CTxInWitness(); w.scriptWitness.stack = stack; wits.append(w)
        t.wit.vtxinwit = wits
        t.rehash()
        self.fee[t.sha256] = fee
        for n, o in enumerate(t.vout):
            self.coins[(t.sha256, n)] = (o.nValue, bytes(o.scriptPubKey))
        return t

    def mine(self, txs=(), branch="main", coinbase=None, spk=None, ntime=None):
        h = self.height + 1
        fees = sum(self.fee[t.sha256] for t in txs)
        cb = coinbase or create_coinbase(h, script_pubkey=spk, fees=fees)
        b = create_block(self.tip, cb, ntime or GEN_TIME + 600 * h, version=4, txlist=list(txs))
        if any(not t.wit.is_null() for t in txs):
            add_witness_commitment(b)
        b.solve()
        got = cli("submitblock", b.serialize().hex())
        assert got in ("", "null"), f"Core refused {branch} block {h}: {got}"
        assert cli("getbestblockhash") == b.hash, f"{branch} block {h} is not the tip"
        b.vtx[0].rehash()
        for n, o in enumerate(b.vtx[0].vout):
            self.coins[(b.vtx[0].sha256, n)] = (o.nValue, bytes(o.scriptPubKey))
        self.blocks.append((branch, h, b))
        self.tip, self.height = b.sha256, h
        return b

    def note(self, name, **kv):
        self.sc[name] = kv


def h(x):
    return f"{x:064x}"


def op(t, n):
    return f"{t.hash}:{n}"


def build(c):
    cbs = {}           # height -> coinbase txid int (main branch)

    # 1..101: coinbase-only blocks, to the miner keys in turn (P2PK both forms, P2PKH, P2WPKH, P2SH-P2WPKH, P2TR).
    # Block 2's coinbase script reads as height 105 (a pre-BIP34 coinbase indicating a later height): the guest tracks
    # it in the BIP30 tree and lets it repeat at exactly 105 once fully spent. A coinbase WITHOUT such a push (the plan's
    # first draft) cannot repeat on this guest: an untracked spent coinbase is SPENT, refused (coinbase-smt bip30.rs:
    # "there the guest is stricter than Core").
    for hh in range(1, 102):
        spk = MINER[TYPES[(hh - 1) % 6]].spk(TYPES[(hh - 1) % 6])
        cb = None
        if hh == 2:
            cb = create_coinbase(2, script_pubkey=spk)
            cb.vin[0].scriptSig = CScript([105])
            cb.rehash()
        b = c.mine(coinbase=cb, spk=spk)
        cbs[hh] = b.vtx[0]
    c.note("coinbase_types", heights="1..101", rule="coinbase h pays MINER[TYPES[(h-1) % 6]]", types=TYPES)
    c.note("tracked_coinbase", height=2, txid=cbs[2].hash, script_sig=cbs[2].vin[0].scriptSig.hex(), indicates=105,
           spent_at=102, repeated_at=105, why="pre-BIP34 coinbase indicating a later height: tracked, repeatable at 105")

    # 102: spends of the two mature coinbases (1: P2PK compressed; 2: P2PK uncompressed, the tracked one).
    a = c.tx([(cbs[1].sha256, 0)], [(100_000_000, ALICE["p2pk_c"].spk("p2pk_c")), (110_000_000, ALICE["p2pk_u"].spk("p2pk_u")),
                                    (120_000_000, ALICE["p2pkh"].spk("p2pkh")), (130_000_000, ALICE["p2wpkh"].spk("p2wpkh")),
                                    (140_000_000, ALICE["p2sh_p2wpkh"].spk("p2sh_p2wpkh")), (150_000_000, ALICE["p2tr"].spk("p2tr")),
                                    (50_000_000, ANYONE), (25_000_000, ANYONE),
                                    (0, CHANGE.spk("p2wpkh")),                    # 8: a zero-value output
                                    (1_000, bytes([0x51]) * 10_001),               # 9: a 10,001-byte script: unspendable by size
                                    (None, CHANGE.spk("p2wpkh"))])                 # 10: change
    dep = c.tx([(cbs[2].sha256, 0), (a.sha256, 7)],
               [(123_456_789, DEPOSIT.spk("p2wpkh")), (0, bytes(CScript([OP_RETURN, b"hazync-deposit-0102"]))),
                (None, CHANGE.spk("p2wpkh"))])
    d75, d80, d300 = bytes(i % 251 for i in range(75)), bytes((i * 7) % 251 for i in range(80)), bytes((i * 13) % 251 for i in range(300))
    forms = [("op_0", bytes([0x6a, 0x00])),
             ("push75", bytes([0x6a, 75]) + d75),
             ("pushdata1_80", bytes([0x6a, 0x4c, 80]) + d80),
             ("pushdata2_300", bytes([0x6a, 0x4d]) + (300).to_bytes(2, "little") + d300),
             ("pushdata4_5", bytes([0x6a, 0x4e]) + (5).to_bytes(4, "little") + b"hello"),
             ("two_pushes_ab_cd", bytes([0x6a, 2]) + b"ab" + bytes([2]) + b"cd"),
             ("op_1_not_data", bytes([0x6a, 0x51])),
             ("bare", bytes([0x6a]))]
    opr = c.tx([(a.sha256, 10)], [(0, s) for _, s in forms] + [(None, CHANGE.spk("p2wpkh"))])
    c.mine([a, dep, opr], spk=MINER["p2pk_c"].spk("p2pk_c"))
    c.note("spend_types_102", height=102, txid=a.hash, spends=op(cbs[1], 0) + " (P2PK compressed coinbase)",
           outputs={"0..5": "ALICE p2pk_c, p2pk_u, p2pkh, p2wpkh, p2sh_p2wpkh, p2tr (1.0..1.5 BTC)", "6": "OP_TRUE 0.5 BTC (spent by the 64-byte tx at 105)",
                    "7": "OP_TRUE 0.25 BTC (spent by the deposit tx in the same block)", "8": "zero-value P2WPKH (CHANGE)",
                    "9": "10,001-byte script, 1,000 sat: unspendable by size, never in the UTXO set", "10": "change (CHANGE P2WPKH), spent in the same block"})
    c.note("deposit_102", height=102, txid=dep.hash, vout=0, amount=123_456_789, script=DEPOSIT.spk("p2wpkh").hex(),
           op_return_vout=1, op_return_payload=b"hazync-deposit-0102".hex(),
           inputs=[op(cbs[2], 0) + " (the tracked block-2 coinbase, P2PK uncompressed)", op(a, 7) + " (same-block, OP_TRUE)"])
    c.note("op_return_forms_102", height=102, txid=opr.hash, outputs={str(i): n for i, (n, _) in enumerate(forms)},
           payloads={"op_0": "", "push75": d75.hex(), "pushdata1_80": d80.hex(), "pushdata2_300": d300.hex(),
                     "pushdata4_5": b"hello".hex(), "two_pushes_ab_cd": b"abcd".hex(), "op_1_not_data": None, "bare": ""},
           note="payload = the push data concatenated; op_1_not_data is not data-only; bare has no push at all")

    # 103: same-block spend chains (1 hop; 3 hops).
    t0 = c.tx([(cbs[3].sha256, 0)], [(2_000_000_000, CHANGE.spk("p2wpkh")), (1_000_000_000, ANYONE), (None, CHANGE.spk("p2wpkh"))])
    t1 = c.tx([(t0.sha256, 0)], [(None, ALICE["p2wpkh"].spk("p2wpkh"))])
    t2 = c.tx([(t0.sha256, 1)], [(None, ANYONE)])
    t3 = c.tx([(t2.sha256, 0)], [(None, ANYONE)])
    t4 = c.tx([(t3.sha256, 0)], [(None, ALICE["p2tr"].spk("p2tr"))])
    c.mine([t0, t1, t2, t3, t4], spk=MINER["p2pk_u"].spk("p2pk_u"))
    c.note("same_block_103", height=103, root=t0.hash, root_spends=op(cbs[3], 0) + " (P2PKH coinbase)",
           one_hop=[t1.hash], three_hops=[t2.hash, t3.hash, t4.hash],
           created_and_spent_in_block=[op(t0, 0), op(t0, 1), op(t2, 0), op(t3, 0)])

    # 104: 2^4 + 1 = 17 transactions (odd: the Merkle tree duplicates the last hash), a 40-output fan-out.
    fan = c.tx([(cbs[4].sha256, 0)], [(100_000_000, ANYONE if i % 2 else CHANGE.spk("p2wpkh")) for i in range(39)] + [(None, CHANGE.spk("p2wpkh"))])
    kids = []
    for i in range(15):
        kids.append(c.tx([(fan.sha256, 2 * i + 1)], [(30_000_000, ALICE[TYPES[i % 6]].spk(TYPES[i % 6])), (None, ANYONE)]))
    b104 = c.mine([fan] + kids, spk=MINER["p2pkh"].spk("p2pkh"))
    assert len(b104.vtx) == 17
    c.note("odd_count_104", height=104, n_tx=17, fanout=fan.hash, fanout_outputs=40, children=[k.hash for k in kids],
           last_tx=kids[-1].hash, fanout_spends=op(cbs[4], 0) + " (P2WPKH coinbase)")

    # 105: block 2's coinbase again, byte for byte (allowed: the first is fully spent, BIP34 not active below 110), so
    # no witness commitment and only non-witness transactions, all zero-fee (the coinbase is identical, so it claims 50
    # BTC, the subsidy). A real 64-byte transaction (non-witness). A P2PK compressed and a P2PKH spend.
    t64 = c.tx([(a.sha256, 6)], [(None, b"\x51\x51\x51\x51")], fee=0)
    assert len(t64.serialize()) == 64
    s105a = c.tx([(a.sha256, 0)], [(None, ALICE["p2pkh"].spk("p2pkh"))], fee=0)
    s105b = c.tx([(a.sha256, 2)], [(None, ALICE["p2pk_u"].spk("p2pk_u"))], fee=0)
    rep = CTransaction(cbs[2]); rep.rehash()
    b105 = c.mine([t64, s105a, s105b], coinbase=rep)
    assert b105.vtx[0].hash == cbs[2].hash and b105.vtx[0].serialize() == cbs[2].serialize()
    assert b105.vtx[0].wit.is_null() and all(t.wit.is_null() for t in b105.vtx)
    c.note("repeated_coinbase_105", height=105, txid=cbs[2].hash, first_height=2, first_spent_at=102,
           live_coin=op(cbs[2], 0), tracked=True, note="gettxout sees one coin (105's); the first was spent at 102")
    c.note("tx_64_bytes_105", height=105, txid=t64.hash, raw=t64.serialize().hex(), spends=op(a, 6), output_spk="51515151",
           position=1, n_tx=4)
    c.note("legacy_spends_105", height=105, p2pk_c=s105a.hash, p2pkh=s105b.hash)

    # 106..110: spends at varied heights; coins created and spent inside the range (L1 106->107, L2 107->108, L3 109->110);
    # L4 created at 109 and left for the reorg (X).
    t106 = c.tx([(cbs[5].sha256, 0), (cbs[6].sha256, 0), (a.sha256, 4)],
                [(3_000_000_000, CHANGE.spk("p2wpkh")), (500_000_000, ALICE["p2tr"].spk("p2tr")), (None, ALICE["p2sh_p2wpkh"].spk("p2sh_p2wpkh"))])
    c.mine([t106], spk=MINER["p2wpkh"].spk("p2wpkh"))
    t107 = c.tx([(t106.sha256, 0), (cbs[7].sha256, 0)], [(4_000_000_000, ANYONE), (None, CHANGE.spk("p2wpkh"))])
    c.mine([t107], spk=MINER["p2sh_p2wpkh"].spk("p2sh_p2wpkh"))
    t108a = c.tx([(t107.sha256, 0)], [(None, ALICE["p2pk_c"].spk("p2pk_c"))])
    t108b = c.tx([(a.sha256, 5)], [(None, ALICE["p2wpkh"].spk("p2wpkh"))])     # P2TR key path (ALICE)
    c.mine([t108a, t108b], spk=MINER["p2tr"].spk("p2tr"))
    t109 = c.tx([(cbs[9].sha256, 0)], [(1_000_000_000, CHANGE.spk("p2wpkh")), (700_000_000, CHANGE.spk("p2wpkh")), (None, ALICE["p2pk_c"].spk("p2pk_c"))])
    c.mine([t109], spk=MINER["p2pk_c"].spk("p2pk_c"))
    t110 = c.tx([(t109.sha256, 0)], [(None, ALICE["p2sh_p2wpkh"].spk("p2sh_p2wpkh"))])
    c.mine([t110], spk=MINER["p2pk_u"].spk("p2pk_u"))
    c.note("varied_106_110", spends={"106": [t106.hash, "cb5 P2SH-P2WPKH + cb6 P2TR + ALICE P2SH-P2WPKH (102:4)"],
                                      "107": [t107.hash, "L1 + cb7 P2PK compressed"], "108": [t108a.hash, t108b.hash, "L2 (OP_TRUE); ALICE P2TR key path (102:5)"],
                                      "109": [t109.hash, "cb9 P2PKH"], "110": [t110.hash, "L3"]},
           L1={"created": 106, "spent": 107, "outpoint": op(t106, 0)}, L2={"created": 107, "spent": 108, "outpoint": op(t107, 0)},
           L3={"created": 109, "spent": 110, "outpoint": op(t109, 0)}, L4={"created": 109, "outpoint": op(t109, 1), "note": "X of the reorg"})
    fork = (c.tip, c.height, dict(c.coins))

    # Branch A, 111..112: deposit D (to DEPOSIT, with an OP_RETURN), X (L4) spent at 111.
    X = (t109.sha256, 1)
    D = c.tx([(cbs[11].sha256, 0)], [(250_000_000, DEPOSIT.spk("p2wpkh")), (0, bytes(CScript([OP_RETURN, b"hazync-deposit-A111"]))), (None, CHANGE.spk("p2wpkh"))])
    xa = c.tx([X], [(None, CHANGE.spk("p2wpkh"))])
    a111 = c.mine([D, xa], branch="a", spk=MINER["p2pkh"].spk("p2pkh"))
    a112s = c.tx([(cbs[12].sha256, 0)], [(10_000_000, ALICE["p2wpkh"].spk("p2wpkh")), (None, CHANGE.spk("p2wpkh"))])
    a112 = c.mine([a112s], branch="a", spk=MINER["p2wpkh"].spk("p2wpkh"))
    # Branch B, 111'..113' (one block more: more work): D absent, X unspent, Y (104's fan-out output 0) spent instead.
    cli("invalidateblock", a111.hash)
    assert int(cli("getblockcount")) == 110
    c.tip, c.height, c.coins = fork[0], fork[1], dict(fork[2])
    Y = (fan.sha256, 0)
    yb = c.tx([Y], [(None, ALICE["p2tr"].spk("p2tr"))])
    b111 = c.mine([yb], branch="b", spk=MINER["p2sh_p2wpkh"].spk("p2sh_p2wpkh"), ntime=GEN_TIME + 600 * 111 + 1)
    b112 = c.mine(branch="b", spk=MINER["p2tr"].spk("p2tr"), ntime=GEN_TIME + 600 * 112 + 1)
    b113s = c.tx([(cbs[13].sha256, 0)], [(None, ALICE["p2pkh"].spk("p2pkh"))])
    b113 = c.mine([b113s], branch="b", spk=MINER["p2pk_c"].spk("p2pk_c"), ntime=GEN_TIME + 600 * 113 + 1)
    cli("reconsiderblock", a111.hash)           # A is valid again; B keeps the tip by more work
    assert cli("getbestblockhash") == b113.hash
    c.note("reorg", fork_height=110, fork_hash=c.blocks[109][2].hash,
           a={"111": a111.hash, "112": a112.hash}, b={"111": b111.hash, "112": b112.hash, "113": b113.hash},
           D={"txid": D.hash, "block": "a 111", "vout": 0, "amount": 250_000_000, "script": DEPOSIT.spk("p2wpkh").hex(),
              "op_return_vout": 1, "op_return_payload": b"hazync-deposit-A111".hex(), "in_b": False},
           X={"outpoint": op(t109, 1), "created": 109, "spent_in_a": {"height": 111, "txid": xa.hash}, "in_b": "unspent"},
           Y={"outpoint": op(fan, 0), "created": 104, "spent_in_b": {"height": 111, "txid": yb.hash}, "in_a": "unspent"},
           a112_tx=a112s.hash, b113_tx=b113s.hash)


def main():
    global DATA
    out = os.path.abspath(sys.argv[1])
    os.makedirs(out, exist_ok=True)
    DATA = tempfile.mkdtemp(prefix="zapp-regtest-gen-")
    node = subprocess.Popen([BIN, "-regtest", f"-datadir={DATA}", f"-rpcport={PORT}", f"-port={PORT + 1}", "-listen=0",
                             "-testactivationheight=bip34@110", "-txindex=0"], stdout=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                cli("getblockcount"); break
            except RuntimeError:
                time.sleep(0.3)
        print("core:", json.loads(cli("getnetworkinfo"))["subversion"], flush=True)
        c = Chain()
        build(c)
        with open(f"{out}/raw.txt", "w") as f:
            for br, hh, b in c.blocks:
                f.write(f"{br} {hh} {b.hash} {b.serialize().hex()}\n")
        keys = {"MINER": {t: MINER[t].describe([t]) for t in TYPES}, "ALICE": {t: ALICE[t].describe([t]) for t in TYPES},
                "DEPOSIT": DEPOSIT.describe(["p2wpkh"]), "CHANGE": CHANGE.describe(["p2wpkh"]),
                "ANYONE": {"spk": ANYONE.hex(), "note": "OP_TRUE"}}
        json.dump({"note": "keys are sha256(\"hazync zapp-regtest key \" + name) mod n, published on purpose (regtest only)",
                   "core": json.loads(cli("getnetworkinfo"))["subversion"], "bitcoind_args": "-regtest -testactivationheight=bip34@110",
                   "keys": keys, "scenarios": c.sc}, open(f"{out}/scenarios.json", "w"), indent=1, sort_keys=True)
        print(f"GEN_CHAIN_OK {len(c.blocks)} blocks, tip b 113 {c.blocks[-1][2].hash}")
    finally:
        try:
            cli("stop")
        except RuntimeError:
            pass
        node.wait(timeout=60)
        shutil.rmtree(DATA, ignore_errors=True)


main()
