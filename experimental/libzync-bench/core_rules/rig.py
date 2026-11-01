#!/usr/bin/env python3
"""rig.py <regtest host> <dir> [case ...]: the guest's verdict against Bitcoin Core's, block by block.

A real bitcoind on regtest and the regtest guest (HAZYNC_NETWORK=regtest) are given the same crafted blocks, built on
one short valid chain with Core's own test_framework. Core's submitblock answer is the reference. A case passes only if
the guest accepts or rejects with it, and a rejection must come from the guest, not from the host failing to read
the block. A coin that does not exist is not rejected per block by design: `deferred` cases are listed separately,
because they fail at H (ss_set != 0), not at the block.

The guest runs in dev mode: rules execute, nothing is proved.
Env: BITCOIND (default the Core 31.1 used for the 136,432 evidence), HZ_CORE_SRC (a Core tree with test/functional).
"""
import json, os, re, shutil, struct, subprocess, sys, time

CORE_SRC = os.environ.get("HZ_CORE_SRC", os.path.expanduser("~/hzb-r7/bitcoin-core"))
sys.path.insert(0, CORE_SRC + "/test/functional")
from test_framework.blocktools import add_witness_commitment, create_block, create_coinbase  # noqa: E402
from test_framework.messages import COIN, COutPoint, CTransaction, CTxIn, CTxInWitness, CTxOut, hash256, ser_uint256  # noqa: E402
from test_framework.key import ECKey, compute_xonly_pubkey, generate_privkey, sign_schnorr, tweak_add_privkey  # noqa: E402
from test_framework.script import (CScript, LegacySignatureHash, OP_0, OP_1, OP_2, OP_3, OP_CHECKMULTISIG, OP_CHECKSIG, OP_DROP,  # noqa: E402
                                   OP_DUP, OP_ENDIF, OP_EQUAL, OP_EQUALVERIFY, OP_HASH160, OP_IF, OP_NOT, OP_RETURN, OP_TRUE, SIGHASH_ALL,
                                   SIGHASH_DEFAULT, SegwitV0SignatureHash, TaprootSignatureHash, hash160, sha256, taproot_construct)

BIN = os.environ.get("BITCOIND", os.path.expanduser("~/hazync-136432/core/bitcoin-31.1/bin/bitcoind"))
CLI = os.path.join(os.path.dirname(BIN), "bitcoin-cli")
HOST, ROOT = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
ONLY = sys.argv[3:]
DATA, BLOCKS, OUT = ROOT + "/node", ROOT + "/blocks", ROOT + "/out"
ANYONE = CScript([OP_TRUE])
BASE = 112   # the valid chain's height: coinbases 1..12 are mature


def cli(*a):
    # Through stdin: a block's hex can be longer than a command line.
    r = subprocess.run([CLI, "-regtest", f"-datadir={DATA}", "-rpcport=18990", "-stdin", str(a[0])],
                       input="".join(f"{x}\n" for x in a[1:]), capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(r.stderr)
    return r.stdout.strip()


class Chain:
    def __init__(self):
        self.times = []                     # block times, index = height - 1 (genesis not included)
        self.gen_time = 1296688602
        self.tip = int(cli("getbestblockhash"), 16)
        self.height = 0
        self.utxo = {}                      # (txid int, n) -> (value, spk, height, is_coinbase, creation mtp)

    def mtp(self, h):
        """MTP of the block AT height h (median of h-10..h)."""
        ts = ([self.gen_time] + self.times)[max(0, h - 10):h + 1]
        return sorted(ts)[len(ts) // 2]

    def recent(self):
        return list(reversed(([self.gen_time] + self.times)[-11:]))

    def next_time(self):
        return ([self.gen_time] + self.times)[-1] + 1

    def block(self, txs=(), fees=0, coinbase=None, ntime=None, version=4, height=None):
        h = (self.height + 1) if height is None else height
        cb = coinbase or create_coinbase(h, script_pubkey=ANYONE, fees=fees)
        b = create_block(self.tip, cb, ntime or self.next_time(), version=version, txlist=list(txs))
        add_witness_commitment(b)
        b.solve()
        return b

    def accept(self, b):
        h = self.height + 1
        for i, t in enumerate(b.vtx):
            t.rehash()
            for n, o in enumerate(t.vout):
                if not (o.scriptPubKey[:1] == bytes([OP_RETURN])):
                    self.utxo[(t.sha256, n)] = (o.nValue, bytes(o.scriptPubKey), h, i == 0, self.mtp(h - 1))
            if i:
                for x in t.vin:
                    self.utxo.pop((x.prevout.hash, x.prevout.n), None)
        self.times.append(b.nTime)
        self.tip = b.sha256
        self.height = h


def rev_hex(n):
    return ser_uint256(n)[::-1].hex()


def to_json(c, b, path, h=None):
    h = c.height + 1 if h is None else h
    txs = []
    made = {}     # coins this block creates, for a spend later in the same block
    for t in b.vtx[1:]:
        t.rehash()
        for n, o in enumerate(t.vout):
            made[(t.sha256, n)] = (o.nValue, bytes(o.scriptPubKey), h, False, c.mtp(h - 1))
    for t in b.vtx[1:]:
        pv = []
        for x in t.vin:
            u = c.utxo.get((x.prevout.hash, x.prevout.n)) or made.get((x.prevout.hash, x.prevout.n))
            if u is None:   # no such coin: the guest is given a plausible one, which is exactly the deferred case
                u = (0, bytes(ANYONE), 1, False, 0)
            pv.append({"value": u[0], "spk": u[1].hex(), "coin_height": u[2], "coin_is_coinbase": int(u[3]), "coin_mtp": u[4]})
        txs.append({"raw": t.serialize().hex(), "prevouts": pv})
    json.dump({"height": h, "bits": b.nBits, "time": b.nTime, "nonce": b.nNonce, "version": b.nVersion,
               "prev": rev_hex(b.hashPrevBlock), "merkle": rev_hex(b.hashMerkleRoot),
               "coinbase_hex": b.vtx[0].serialize().hex(), "txs": txs, "recent_times": c.recent()}, open(path, "w"))


def spend(c, coins, outs, seq=0xffffffff, version=2, locktime=0):
    t = CTransaction()
    t.version = version
    t.nLockTime = locktime
    t.vin = [CTxIn(COutPoint(a, b), b"", seq if isinstance(seq, int) else seq[i]) for i, (a, b) in enumerate(coins)]
    t.vout = [CTxOut(v, s) for v, s in outs]
    t.rehash()
    return t


def mature(c, k=1):
    r = [o for o, u in sorted(c.utxo.items(), key=lambda kv: kv[1][2]) if u[2] + 100 <= c.height + 1 and u[1] == bytes(ANYONE)]
    return r[:k]


def value(c, coins):
    return sum(c.utxo[x][0] for x in coins)


def out(v):
    return (v, bytes(ANYONE))


def wit(t, stacks):
    for s in stacks:
        w = CTxInWitness()
        w.scriptWitness.stack = s
        t.wit.vtxinwit.append(w)
    t.rehash()


# ---- cases: each returns (block, json-height override or None). Core's reject reason is read, not assumed. --------
def regrind(b):
    b.hashMerkleRoot = b.calc_merkle_root()
    b.solve()
    return b


def c_valid_empty(c): return c.block()
def c_valid_spend(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x) - 1000)])], fees=1000)
def c_valid_zero_fee(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))])])
def c_cb_missing(c):
    x = mature(c); t = spend(c, x, [out(value(c, x))]); b = c.block(); b.vtx = [t]; return regrind(b)
def c_cb_multiple(c):
    b = c.block(); b.vtx.append(create_coinbase(c.height + 1, script_pubkey=ANYONE)); b.vtx[-1].vin[0].scriptSig = CScript([b"xx"]); b.vtx[-1].rehash(); return regrind(b)
def c_cb_overpay(c):
    return c.block(coinbase=create_coinbase(c.height + 1, script_pubkey=ANYONE, fees=1))
def c_cb_exact(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x) - 777)])], fees=777)
def c_cb_fee_too_much(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x) - 777)])], fees=778)
def c_cb_script_short(c):
    cb = create_coinbase(c.height + 1, script_pubkey=ANYONE); cb.vin[0].scriptSig = CScript([b"\x01"])[:1]; cb.rehash(); return c.block(coinbase=cb)
def c_cb_script_long(c):
    cb = create_coinbase(c.height + 1, script_pubkey=ANYONE); cb.vin[0].scriptSig = bytes(cb.vin[0].scriptSig) + b"\0" * 110; cb.rehash(); return c.block(coinbase=cb)
def c_cb_wrong_height(c):
    return c.block(coinbase=create_coinbase(c.height + 2, script_pubkey=ANYONE))
def c_bad_merkle(c):
    b = c.block(); b.hashMerkleRoot ^= 1; b.solve(); return b
def c_merkle_mutated(c):
    x = mature(c); t = spend(c, x, [out(value(c, x))]); b = c.block([t]); b.vtx.append(t); return regrind(b)
def c_tx_dup_inputs(c):
    x = mature(c); return c.block([spend(c, x + x, [out(value(c, x))])])
def c_tx_no_inputs(c):
    t = CTransaction(); t.version = 2; t.vout = [CTxOut(0, ANYONE)]; t.rehash(); return c.block([t])
def c_tx_no_outputs(c):
    x = mature(c); t = spend(c, x, []); return c.block([t])
def c_tx_negative_out(c):
    x = mature(c); return c.block([spend(c, x, [out(-1)])])
def c_tx_huge_out(c):
    x = mature(c); return c.block([spend(c, x, [out(21_000_000 * COIN + 1)])])
def c_tx_total_huge(c):
    x = mature(c); return c.block([spend(c, x, [out(11_000_000 * COIN), out(11_000_000 * COIN)])])
def c_tx_null_prevout(c):
    t = CTransaction(); t.version = 2; t.vin = [CTxIn(COutPoint(0, 0xffffffff), b"")]; t.vout = [CTxOut(1, ANYONE)]; t.rehash(); return c.block([t])
def c_premature(c):
    young = [o for o, u in c.utxo.items() if u[3] and u[2] > c.height - 50][:1]; return c.block([spend(c, young, [out(value(c, young))])])
def c_exactly_mature(c):
    o = [o for o, u in c.utxo.items() if u[3] and u[2] == c.height + 1 - 100][:1]; return c.block([spend(c, o, [out(value(c, o))])])
def c_one_short_of_mature(c):
    o = [o for o, u in c.utxo.items() if u[3] and u[2] == c.height + 1 - 99][:1]; return c.block([spend(c, o, [out(value(c, o))])])
def c_in_below_out(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x) + 1)])])
def c_missing_coin(c):
    return c.block([spend(c, [(0x1234, 0)], [out(0)])])
def c_double_spend_in_block(c):
    x = mature(c); return c.block([spend(c, x, [out(1)]), spend(c, x, [out(2)])])
def c_chain_in_block(c):
    x = mature(c); a = spend(c, x, [out(value(c, x))]); b = spend(c, [(a.sha256, 0)], [out(value(c, x))]); return c.block([a, b])
def c_spend_later_in_block(c):
    x = mature(c); a = spend(c, x, [out(value(c, x))]); b = spend(c, [(a.sha256, 0)], [out(value(c, x))]); return c.block([b, a])
def c_locktime_height_future(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0, locktime=c.height + 1)])
def c_locktime_height_ok(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0, locktime=c.height)])
def c_locktime_final_seq(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], locktime=c.height + 1000)])
def c_locktime_time_mtp(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0, locktime=c.mtp(c.height))])
def c_locktime_time_past(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0, locktime=c.mtp(c.height) - 1)])
def c_bip68_height_unmet(c):
    x = mature(c); age = c.height + 1 - c.utxo[x[0]][2]; return c.block([spend(c, x, [out(value(c, x))], seq=age + 1)])
def c_bip68_height_met(c):
    x = mature(c); age = c.height + 1 - c.utxo[x[0]][2]; return c.block([spend(c, x, [out(value(c, x))], seq=age)])
def c_bip68_disabled(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0x80000000 | 0xffff)])
def c_bip68_v1_ignored(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0xffff, version=1)])
def c_bip68_uint_version(c):
    x = mature(c); return c.block([spend(c, x, [out(value(c, x))], seq=0xffff, version=0xffffffff)])
def c_bip68_time_met(c):
    x = mature(c); u = c.utxo[x[0]]; e = c.mtp(c.height) - u[4]; return c.block([spend(c, x, [out(value(c, x))], seq=(1 << 22) | (e >> 9))])
def c_bip68_time_unmet(c):
    x = mature(c); u = c.utxo[x[0]]; e = c.mtp(c.height) - u[4]; return c.block([spend(c, x, [out(value(c, x))], seq=(1 << 22) | ((e >> 9) + 1))])
def c_time_equal_mtp(c):
    return c.block(ntime=c.mtp(c.height))
def c_time_above_mtp(c):
    return c.block(ntime=c.mtp(c.height) + 1)
def c_bad_version(c):
    return c.block(version=3)
def c_version_4(c):
    return c.block(version=0x20000000)
def c_high_hash(c):
    b = c.block()
    while int.from_bytes(hash256(b.serialize()[:80])[::-1], "big") <= (0x7fffff << (8 * 29)):
        b.nNonce += 1
    b.rehash(); b.sha256 = int.from_bytes(hash256(b.serialize()[:80])[::-1], "big"); return b
def c_bad_bits(c):
    b = c.block(); b.nBits = 0x207ffffe; b.solve(); return b
def c_nonce_too_big_witness(c):   # a new header: witness data does not change it, and Core would answer from its cache
    b = c.block(ntime=c.next_time() + 1); b.vtx[0].wit.vtxinwit[0].scriptWitness.stack = [b"\0" * 33]; b.vtx[0].rehash(); return b
def c_witness_mismatch(c):
    b = c.block(ntime=c.next_time() + 2); b.vtx[0].wit.vtxinwit[0].scriptWitness.stack = [b"\1" * 32]; b.vtx[0].rehash(); return b
def c_no_commit_no_witness(c):
    b = create_block(c.tip, create_coinbase(c.height + 1, script_pubkey=ANYONE), c.next_time() + 3, version=4); b.solve(); return b
def _p2wsh(c):
    x = mature(c); v = value(c, x); spk = CScript([OP_0, sha256(bytes(ANYONE))])
    a = spend(c, x, [(v, bytes(spk))]); return a, v, spk
def c_valid_p2wsh(c):
    a, v, _ = _p2wsh(c); b = spend(c, [(a.sha256, 0)], [out(v)]); wit(b, [[bytes(ANYONE)]]); return c.block([a, b])
def c_witness_no_commit(c):
    a, v, _ = _p2wsh(c); t = spend(c, [(a.sha256, 0)], [out(v)]); wit(t, [[bytes(ANYONE)]])
    b = create_block(c.tip, create_coinbase(c.height + 1, script_pubkey=ANYONE), c.next_time(), version=4, txlist=[a, t]); b.solve(); return b
def c_script_fails(c):
    x = mature(c); t = spend(c, x, [out(value(c, x))]); t.vin[0].scriptSig = CScript([OP_RETURN]); t.rehash(); return c.block([t])
def c_sigops_limit(c):
    x = mature(c); s = CScript([OP_CHECKSIG] * 20001); return c.block([spend(c, x, [(value(c, x), bytes(s))])])
def c_sigops_at_limit(c):
    n = 20000 - 1 - 2   # 80,000 cost less the coinbase's own and the commitment's
    x = mature(c); s = CScript([OP_CHECKSIG] * n); return c.block([spend(c, x, [(value(c, x), bytes(s))])])
def c_oversize_weight(c):
    x = mature(c); s = bytes(CScript([OP_RETURN])) + b"\0" * 1_100_000; return c.block([spend(c, x, [(value(c, x), s)])])
def c_split_tx(c):
    # Many inputs, so that with HAZYNC_CHUNKS=2 the transaction spans two chunks: the second carries the bad lock.
    x = mature(c, 8); ages = [c.height + 1 - c.utxo[o][2] for o in x]; seqs = [0xffffffff] * 7 + [ages[7]]
    return c.block([spend(c, x, [out(value(c, x))], seq=seqs)])
def c_split_tx_bad(c):
    x = mature(c, 8); ages = [c.height + 1 - c.utxo[o][2] for o in x]; seqs = [0xffffffff] * 7 + [ages[7] + 1]
    return c.block([spend(c, x, [out(value(c, x))], seq=seqs)])


# ---- signatures: the batch verifier and its hints --------------------------------------------------------------
N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141


def key():
    k = ECKey(); k.generate(); return k, k.get_pubkey().get_bytes()


def fund(c, spk, n=1):
    x = mature(c); v = value(c, x) // n
    return spend(c, x, [(v, bytes(spk))] * n), v


def der_parts(sig):
    rl = sig[3]; r = int.from_bytes(sig[4:4 + rl], "big"); sl = sig[5 + rl]; return r, int.from_bytes(sig[6 + rl:6 + rl + sl], "big")


def der(r, s):
    def e(x):
        b = x.to_bytes(32, "big").lstrip(b"\0")
        return b"\0" + b if b[0] & 0x80 else b
    a, b = e(r), e(s)
    return bytes([0x30, 4 + len(a) + len(b), 2, len(a)]) + a + bytes([2, len(b)]) + b


def p2wpkh(c, mangle=lambda s: s):
    k, pub = key(); a, v = fund(c, CScript([OP_0, hash160(pub)])); t = spend(c, [(a.sha256, 0)], [out(v)])
    code = CScript([OP_DUP, OP_HASH160, hash160(pub), OP_EQUALVERIFY, OP_CHECKSIG])
    sig = mangle(k.sign_ecdsa(SegwitV0SignatureHash(code, t, 0, SIGHASH_ALL, v)))
    wit(t, [[sig + bytes([SIGHASH_ALL]), pub]]); return c.block([a, t])
def c_sig_p2wpkh(c): return p2wpkh(c)
def c_sig_p2wpkh_bad(c):
    return p2wpkh(c, lambda s: s[:-1] + bytes([s[-1] ^ 1]))
def c_sig_high_s(c):
    def f(s):
        a, b = der_parts(s); return der(a, N - b if b < N // 2 else b)
    return p2wpkh(c, f)
def c_sig_not_der(c):
    return p2wpkh(c, lambda s: s + b"\0")
def c_sig_zero_r(c):
    return p2wpkh(c, lambda s: der(1, der_parts(s)[1]))
def p2pkh(c, other_key=False):
    k, pub = key(); spk = CScript([OP_DUP, OP_HASH160, hash160(pub), OP_EQUALVERIFY, OP_CHECKSIG]); a, v = fund(c, spk)
    t = spend(c, [(a.sha256, 0)], [out(v)]); h, _ = LegacySignatureHash(spk, t, 0, SIGHASH_ALL)
    sig = (key()[0] if other_key else k).sign_ecdsa(h) + bytes([SIGHASH_ALL]); t.vin[0].scriptSig = CScript([sig, pub]); t.rehash(); return c.block([a, t])
def c_sig_p2pkh(c): return p2pkh(c)
def c_sig_p2pkh_other_key(c): return p2pkh(c, True)
def multi(c, pick, dummy=OP_0):
    ks = [key() for _ in range(3)]; spk = CScript([OP_2, ks[0][1], ks[1][1], ks[2][1], OP_3, OP_CHECKMULTISIG]); a, v = fund(c, spk)
    t = spend(c, [(a.sha256, 0)], [out(v)]); h, _ = LegacySignatureHash(spk, t, 0, SIGHASH_ALL)
    t.vin[0].scriptSig = CScript([dummy] + [ks[i][0].sign_ecdsa(h) + bytes([SIGHASH_ALL]) for i in pick]); t.rehash(); return c.block([a, t])
def c_multisig_ok(c): return multi(c, (0, 2))
def c_multisig_order(c): return multi(c, (2, 0))
def c_multisig_dummy(c): return multi(c, (0, 1), OP_1)
def must_fail(c, good=False):
    # A script that needs its signature to be WRONG. The hint says "invalid"; the guest must not take that on trust.
    k, pub = key(); spk = CScript([pub, OP_CHECKSIG, OP_NOT]); a, v = fund(c, spk); t = spend(c, [(a.sha256, 0)], [out(v)])
    h, _ = LegacySignatureHash(spk, t, 0, SIGHASH_ALL); sig = k.sign_ecdsa(h)
    t.vin[0].scriptSig = CScript([(sig if good else sig[:-1] + bytes([sig[-1] ^ 1])) + bytes([SIGHASH_ALL])]); t.rehash(); return c.block([a, t])
def c_sig_must_fail(c): return must_fail(c)
def c_sig_must_fail_but_good(c): return must_fail(c, True)
def tap_key(c, flip=False, annex=None, coin=None, txs_only=False):
    p = generate_privkey(); x, _ = compute_xonly_pubkey(p); info = taproot_construct(x)
    if coin:
        a = spend(c, [coin], [(value(c, [coin]), bytes(info.scriptPubKey))]); v = value(c, [coin])
    else:
        a, v = fund(c, info.scriptPubKey)
    t = spend(c, [(a.sha256, 0)], [out(v)]); m = TaprootSignatureHash(t, [CTxOut(v, info.scriptPubKey)], SIGHASH_DEFAULT, 0, annex=annex)
    sig = sign_schnorr(tweak_add_privkey(p, info.tweak), m)
    wit(t, [[sig[:-1] + bytes([sig[-1] ^ 1]) if flip else sig] + ([annex] if annex else [])])
    return [a, t] if txs_only else c.block([a, t])
def c_taproot_key(c): return tap_key(c)
def c_taproot_key_bad(c): return tap_key(c, True)
def c_weight_under(c): return tap_key(c, annex=b"\x50" + b"\0" * 3_990_000)
def c_weight_over(c):   # two transactions: the host's parser refuses one of 4 MB before the guest sees it
    x = mature(c, 2)
    return c.block(tap_key(c, annex=b"\x50" + b"\0" * 2_100_000, coin=x[0], txs_only=True)
                   + tap_key(c, annex=b"\x50" + b"\0" * 2_100_000, coin=x[1], txs_only=True))
def tap_script(c, parity=False, wrong_sig=False):
    p = generate_privkey(); x, _ = compute_xonly_pubkey(p); lk = generate_privkey(); lx, _ = compute_xonly_pubkey(lk)
    info = taproot_construct(x, [("l", CScript([lx, OP_CHECKSIG]))]); leaf = info.leaves["l"]; a, v = fund(c, info.scriptPubKey)
    t = spend(c, [(a.sha256, 0)], [out(v)])
    m = TaprootSignatureHash(t, [CTxOut(v, info.scriptPubKey)], SIGHASH_DEFAULT, 0, scriptpath=True, leaf_script=leaf.script, leaf_ver=leaf.version)
    sig = sign_schnorr(generate_privkey() if wrong_sig else lk, m)
    cb = bytes([leaf.version + (info.negflag ^ (1 if parity else 0))]) + info.internal_pubkey + leaf.merklebranch
    wit(t, [[sig, bytes(leaf.script), cb]]); return c.block([a, t])
def c_taproot_script(c): return tap_script(c)
def c_taproot_tweak_parity(c): return tap_script(c, parity=True)
def c_taproot_script_bad_sig(c): return tap_script(c, wrong_sig=True)


# ---- sigops through P2SH and witness, counted without ever executing -------------------------------------------
HEAVY = CScript([OP_0, OP_IF] + [OP_CHECKMULTISIG] * 197 + [OP_ENDIF, OP_1])   # 197 * 20 sigops, never run


def many(c, n, p2sh):
    spk = CScript([OP_HASH160, hash160(bytes(HEAVY)), OP_EQUAL]) if p2sh else CScript([OP_0, sha256(bytes(HEAVY))])
    a, v = fund(c, spk, n); t = spend(c, [(a.sha256, i) for i in range(n)], [out(v * n)])
    if p2sh:
        for x in t.vin:
            x.scriptSig = CScript([bytes(HEAVY)])
        t.rehash()
    else:
        wit(t, [[bytes(HEAVY)]] * n)
    return c.block([a, t])
def c_sigops_witness_under(c): return many(c, 20, False)
def c_sigops_witness_over(c): return many(c, 21, False)
def c_sigops_p2sh_under(c): return many(c, 5, True)
def c_sigops_p2sh_over(c): return many(c, 6, True)
def c_tx_64_bytes(c):
    x = mature(c); t = spend(c, x, [(value(c, x), b"\x51\x51\x51\x51")]); assert len(t.serialize()) == 64; return c.block([t])


CASES = [(k[2:], v) for k, v in list(globals().items()) if k.startswith("c_")]
DEFERRED = {"missing_coin"}
TWO_CHUNKS = {"split_tx", "split_tx_bad"}


def guest(c, name, b, height):
    d = f"{BLOCKS}/{name}"
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    for h in range(1, c.height + 1):
        os.symlink(f"{BLOCKS}/main/block_{h}.json", f"{d}/block_{h}.json")
    to_json(c, b, f"{d}/block_{height}.json")
    o = f"{OUT}/{name}"
    shutil.rmtree(o, ignore_errors=True)
    env = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_RIG="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS="2" if name in TWO_CHUNKS else "1")
    if name in TWO_CHUNKS:
        env["HAZYNC_CHUNK_PACK"] = "count"   # split by input count: 4 + 4, the bad lock in the second run
    r = subprocess.run([HOST, "libzync-prove-range", str(height), str(height), d, f"{BLOCKS}/anchor.json", o],
                       env=env, capture_output=True, text=True, errors="replace", timeout=900)
    t = r.stdout + r.stderr
    if name == "split_tx":
        assert ": 2 chunks" in t, "split_tx did not span two chunks: " + t[-300:]
    p = [l for l in t.splitlines() if "panicked" in l or "agg-lite: block INVALID" in l or "Guest panicked" in l or "failed" in l.lower()]
    why = " ".join(t[t.find("block INVALID"):][:160].split()) if "block INVALID" in t else (p[-1][:150] if p else "")
    in_guest = "Guest panicked" in t or "block INVALID" in t or "prove chunk" in t
    return r.returncode == 0, why, in_guest, t


def main():
    shutil.rmtree(DATA, ignore_errors=True)
    os.makedirs(DATA); os.makedirs(BLOCKS + "/main", exist_ok=True); os.makedirs(OUT, exist_ok=True)
    subprocess.Popen([BIN, "-regtest", f"-datadir={DATA}", "-rpcport=18990", "-port=18991", "-listen=0", "-txindex=0", "-fallbackfee=0.0001"], stdout=subprocess.DEVNULL)
    for _ in range(100):
        try:
            cli("getblockcount"); break
        except RuntimeError:
            time.sleep(0.3)
    try:
        run()
    finally:
        cli("stop")


def run():
    print("core:", cli("getnetworkinfo").split('"subversion": "')[1].split('"')[0], flush=True)
    c = Chain()
    for _ in range(BASE):
        b = c.block()
        assert cli("submitblock", b.serialize().hex()) in ("", "null"), "base chain"
        to_json(c, b, f"{BLOCKS}/main/block_{c.height + 1}.json")
        c.accept(b)
    # The anchor: the guest's own state at BASE, from genesis. Reused while the guest image is the same.
    anchor = f"{BLOCKS}/anchor.json"
    if not os.path.exists(anchor):
        o = f"{OUT}/base"
        env = dict(os.environ, RISC0_DEV_MODE="1", HAZYNC_PROFILE_LAYOUT="cost", HAZYNC_CHUNKS="1")
        r = subprocess.run([HOST, "libzync-prove-range", "1", str(BASE), BLOCKS + "/main", "genesis", o], env=env, capture_output=True, text=True, errors="replace")
        assert r.returncode == 0, (r.stdout + r.stderr)[-800:]
        shutil.copy(f"{o}/state_{BASE}.json", anchor)
    res = []
    for name, f in CASES:
        if ONLY and name not in ONLY:
            continue
        b = f(c)
        b_hash = rev_hex(int.from_bytes(hash256(b.serialize()[:80]), "little"))
        try:
            core = cli("submitblock", b.serialize().hex())
        except RuntimeError as e:
            assert "Block decode failed" in str(e), e
            core = "decode-failed"
        core_ok = core in ("", "null")
        if core_ok:
            cli("invalidateblock", b_hash)
        g_ok, why, in_guest, _ = guest(c, name, b, c.height + 1)
        if name in DEFERRED:
            state = "DEFERRED-OK" if (not core_ok and g_ok) else "CHECK"
        elif core_ok != g_ok:
            state = "DISAGREE"
        elif not g_ok and not in_guest and core != "decode-failed":
            state = "HOST-FAIL"
        else:
            state = "AGREE"
        res.append(state)
        print(f"{state:12} {name:24} core={'accept' if core_ok else core:38} guest={'accept' if g_ok else 'reject'} {'' if g_ok else why[:110]}", flush=True)
    bad = [s for s in res if s not in ("AGREE", "DEFERRED-OK")]
    print(f"RIG {len(res)} cases, {res.count('AGREE')} agree, {res.count('DEFERRED-OK')} deferred, {len(bad)} to look at")
    return 1 if bad else 0


main()
