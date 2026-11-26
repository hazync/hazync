#!/usr/bin/env python3
"""make_fixture.py <zapp-regtest dir> <test_framework parent dir> <out dir>: the zeroconf zAPP's regtest fixture
(engine/results/zapp_utreexo, "zeroconf: the plan"). A transaction T spending two harness coins unspent at block 110
(ALICE's P2WPKH output of 102 and her P2PKH output of 105, keys published in chain/scenarios.json; Core's own
gettxout at 110 says both are unspent), signed with Core's test_framework (RFC 6979 nonces: the same bytes every run), and the block fixture for height 111 on
top of 110 holding T alone (fetch_template_rpc.py's schema: a dummy coinbase never proved, the header fields a
speculative chunk does not read as placeholders). Writes <out>/T.json (txid, wtxid, raw, fee, inputs) and
<out>/template_111.json."""
import json, os, sys, hashlib
ZR, TF, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
sys.path.insert(0, TF)
from test_framework.key import ECKey
from test_framework.messages import COutPoint, CTransaction, CTxIn, CTxInWitness, CTxOut
from test_framework.script import CScript, LegacySignatureHash, SegwitV0SignatureHash, SIGHASH_ALL
from test_framework.script_util import keyhash_to_p2pkh_script
os.makedirs(OUT, exist_ok=True)
sc = json.load(open(f"{ZR}/chain/scenarios.json"))["keys"]["ALICE"]
utxo = json.load(open(f"{ZR}/oracle/snap_110.json"))["gettxout"]
blk = lambda h: json.load(open(f"{ZR}/blocks/main/block_{h}.json"))
def h160(b): return hashlib.new("ripemd160", hashlib.sha256(b).digest()).digest()
def key(kind):
    k = ECKey(); k.set(bytes.fromhex(sc[kind]["secret"]), True); return k
def mtp(h):  # median time past of block h (its 11 blocks up to h)
    t = sorted(blk(x)["time"] for x in range(max(1, h - 10), h + 1)); return t[len(t) // 2]
ins = [("bffc2f43cbf691028de7155a4c84e5e83be35dee221d20136b2b5e68f01434dc", 3, "p2wpkh"),
       ("aa970d975d882c16a9bf7bd8d552f345242cfad2bf54b899370eb3083cc4e125", 0, "p2pkh")]
prev = []
for txid, vout, kind in ins:
    u = utxo[f"{txid}:{vout}"]
    assert u is not None, f"{txid}:{vout} is not unspent at 110 (Core's gettxout)"
    prev.append((txid, vout, kind, u))
tx = CTransaction()
tx.version = 2
tx.vin = [CTxIn(COutPoint(int(t, 16), v), b"", 0xfffffffd) for t, v, _, _ in prev]
total = sum(u["value_sat"] for *_, u in prev)
fee = 10_000
dest = bytes.fromhex(sc["p2wpkh"]["scripts"]["p2wpkh"]["spk"])
tx.vout = [CTxOut(total - fee, dest)]
tx.wit.vtxinwit = [CTxInWitness() for _ in tx.vin]
for i, (t, v, kind, u) in enumerate(prev):
    k = key(kind); pub = k.get_pubkey().get_bytes()
    spk = bytes.fromhex(u["spk"])
    if kind == "p2wpkh":
        assert spk == b"\x00\x14" + h160(pub), "the coin's script is not this key's P2WPKH"
        sh = SegwitV0SignatureHash(keyhash_to_p2pkh_script(h160(pub)), tx, i, SIGHASH_ALL, u["value_sat"])
        tx.wit.vtxinwit[i].scriptWitness.stack = [k.sign_ecdsa(sh, rfc6979=True) + bytes([SIGHASH_ALL]), pub]
    else:
        if spk != keyhash_to_p2pkh_script(h160(pub)):   # the harness may pay the uncompressed key's hash
            k2 = ECKey(); k2.set(bytes.fromhex(sc[kind]["secret"]), False); pub = k2.get_pubkey().get_bytes(); k = k2
        assert spk == keyhash_to_p2pkh_script(h160(pub)), "the coin's script is not this key's P2PKH"
        sh, _ = LegacySignatureHash(CScript(spk), tx, i, SIGHASH_ALL)
        tx.vin[i].scriptSig = CScript([k.sign_ecdsa(sh, rfc6979=True) + bytes([SIGHASH_ALL]), pub])
raw = tx.serialize_with_witness().hex()
info = {"txid": tx.rehash(), "wtxid": tx.getwtxid(), "raw": raw, "fee": fee, "vsize": tx.get_vsize(), "weight": tx.get_weight(),
        "inputs": [{"outpoint": f"{t}:{v}", "kind": kind, "height": u["height"], "value": u["value_sat"]} for t, v, kind, u in prev]}
json.dump(info, open(f"{OUT}/T.json", "w"), indent=1)
# the block fixture for 111 (fetch_template_rpc.py's fields). Its difficulty epoch starts at genesis (block 0): the
# harness's block times are genesis + 600 h, and regtest's genesis time is 1296688602.
GENESIS_TIME = blk(1)["time"] - 600
assert GENESIS_TIME == 1296688602, GENESIS_TIME
# the block fixture for 111 (fetch_template_rpc.py's fields)
H, b110 = 111, blk(110)
hp = H.to_bytes((H.bit_length() + 8) // 8, "little"); sig = bytes([len(hp)]) + hp
cb = (b"\x01\x00\x00\x00" + b"\x01" + b"\x00" * 32 + b"\xff\xff\xff\xff" + bytes([len(sig)]) + sig + b"\xff\xff\xff\xff"
      + b"\x01" + (0).to_bytes(8, "little") + b"\x01\x6a" + b"\x00" * 4).hex()
tmpl = {"height": H, "version": 0x20000000, "time": b110["time"] + 600, "bits": b110["bits"], "nonce": 0,
        "prev": json.load(open(f"{ZR}/oracle/snap_110.json"))["hash"], "merkle": "00" * 32, "coinbase_hex": cb,
        "recent_times": [blk(x)["time"] for x in range(100, 111)], "epoch_start": GENESIS_TIME,
        "prev_time": b110["time"], "prev_bits": b110["bits"], "speculative": True,
        "txs": [{"raw": raw, "prevouts": [{"value": u["value_sat"], "spk": u["spk"], "coin_height": u["height"],
                                            "coin_is_coinbase": 1 if u["coinbase"] else 0, "coin_mtp": mtp(u["height"] - 1)}
                                           for *_, u in prev]}]}
json.dump(tmpl, open(f"{OUT}/template_111.json", "w"))
print(f"T {info['txid']} (wtxid {info['wtxid'][:16]}..), {len(prev)} inputs, fee {fee}, weight {info['weight']}; template for 111 written")
