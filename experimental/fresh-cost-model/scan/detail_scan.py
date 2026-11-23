"""Scan B: decode sampled blocks (getblock verbosity 3) and count primitive work.

Usage: detail_scan.py OUT.jsonl HEIGHTS_FILE [WORKERS]
One JSON line per block. Resumable (skips heights already in OUT).

Counts are of WORK A VALIDATOR MUST DO, derived from the block bytes:
  ecdsa / schnorr   signatures present in spends (DER-shaped items / 64-65 byte taproot items)
  ecdsa_cms_upper   CHECKMULTISIG worst-case attempts (n keys) for multisig spends, else = sigs
  sha_c_*           SHA-256 compression-function calls, split by purpose
  ripemd            RIPEMD-160 calls (hash160 of pubkeys / redeem scripts)
  utxo_spend / utxo_create
  script_bytes      bytes of script the interpreter walks (scriptSig + spk + redeem/witness/tap script)
"""
import json, os, sys, threading, queue
from collections import Counter
from rpc import RPC

def sha_c(n):            # compressions for single SHA-256 of n bytes
    return (n + 9 + 63) // 64
def dsha_c(n):           # double SHA-256
    return sha_c(n) + 1
def tagged_c(n):         # tagged hash with precomputed tag midstate (Core does this)
    return (n + 9 + 63) // 64
def varint_len(n):
    return 1 if n < 0xfd else 3 if n <= 0xffff else 5

def pushes(script):
    """Return list of pushed data items (None for non-push opcodes). Tolerant of truncation."""
    out, i, n = [], 0, len(script)
    while i < n:
        op = script[i]; i += 1
        if 1 <= op <= 75:
            out.append(script[i:i + op]); i += op
        elif op == 76 and i < n:
            l = script[i]; i += 1; out.append(script[i:i + l]); i += l
        elif op == 77 and i + 1 < n:
            l = int.from_bytes(script[i:i + 2], "little"); i += 2; out.append(script[i:i + l]); i += l
        elif op == 78 and i + 3 < n:
            l = int.from_bytes(script[i:i + 4], "little"); i += 4; out.append(script[i:i + l]); i += l
        elif op == 0:
            out.append(b"")
        else:
            out.append(None)
    return out

def is_der(b):
    return b is not None and 9 <= len(b) <= 73 and b[0] == 0x30 and b[1] == len(b) - 3

def multisig_n(script):
    if len(script) >= 3 and script[-1] == 0xae and 0x51 <= script[-2] <= 0x60:
        return script[-2] - 0x50
    return 0

def scan_block(rpc, h):
    bh = rpc.call("getblockhash", h)
    b = rpc.call("getblock", bh, 3)
    c = Counter()
    c["height"] = h
    c["txs"] = len(b["tx"])
    c["size"] = b["size"]; c["weight"] = b["weight"]
    c["sha_c_header"] = dsha_c(80)
    ntx = len(b["tx"])
    any_witness = False
    for tx in b["tx"]:
        size, weight = tx["size"], tx["weight"]
        stripped = (weight - size) // 3
        has_wit = size != stripped
        any_witness |= has_wit
        c["sha_c_txid"] += dsha_c(stripped)
        if has_wit:
            c["sha_c_wtxid"] += dsha_c(size)
        coinbase = "coinbase" in tx["vin"][0]
        outs_ser = 0
        for o in tx["vout"]:
            spk = bytes.fromhex(o["scriptPubKey"]["hex"])
            outs_ser += 8 + varint_len(len(spk)) + len(spk)
            if o["scriptPubKey"]["type"] != "nulldata" and not spk[:1] == b"\x6a":
                c["utxo_create"] += 1
            else:
                c["outputs_unspendable"] += 1
        if coinbase:
            continue
        nin = len(tx["vin"])
        # legacy sighash preimage: tx with every scriptSig blanked, current one = scriptCode (~25 B)
        ss_total = sum(len(v["scriptSig"]["hex"]) // 2 + varint_len(len(v["scriptSig"]["hex"]) // 2) - 1 for v in tx["vin"])
        legacy_preimage = stripped - ss_total + 25 + 4
        c["inputs"] += nin
        c["utxo_spend"] += nin
        uses_v0 = uses_v1 = False
        spks_ser = 0
        for vin in tx["vin"]:
            pv = vin["prevout"]
            spk = bytes.fromhex(pv["scriptPubKey"]["hex"])
            spks_ser += 8 + varint_len(len(spk)) + len(spk)
            t = pv["scriptPubKey"]["type"]
            ss = bytes.fromhex(vin["scriptSig"]["hex"])
            wit = [bytes.fromhex(x) for x in vin.get("txinwitness", [])]
            c["script_bytes"] += len(ss) + len(spk)
            c[f"in_{t}"] += 1
            ecdsa = schnorr = cms_upper = 0
            legacy_sigs = v0_sigs = 0
            v0_scriptcode = 25
            if t in ("pubkey", "pubkeyhash", "multisig", "nonstandard"):
                ds = sum(1 for p in pushes(ss) if is_der(p))
                ecdsa += ds; legacy_sigs += ds
                if t == "pubkeyhash":
                    c["sha_c_hash160"] += 1; c["ripemd"] += 1
                if t == "multisig":
                    cms_upper += max(multisig_n(spk), ds)
                else:
                    cms_upper += ds
            elif t == "scripthash":
                ps = pushes(ss)
                redeem = ps[-1] if ps and ps[-1] is not None else b""
                c["sha_c_hash160"] += sha_c(len(redeem)); c["ripemd"] += 1
                c["script_bytes"] += len(redeem)
                if len(redeem) == 22 and redeem[:2] == b"\x00\x14":          # P2SH-P2WPKH
                    c["in_p2sh_p2wpkh"] += 1
                    ecdsa += 1; v0_sigs += 1; cms_upper += 1; uses_v0 = True
                    c["sha_c_hash160"] += 1; c["ripemd"] += 1
                elif len(redeem) == 34 and redeem[:2] == b"\x00\x20":        # P2SH-P2WSH
                    c["in_p2sh_p2wsh"] += 1
                    ws = wit[-1] if wit else b""
                    c["sha_c_wscript"] += sha_c(len(ws)); c["script_bytes"] += len(ws)
                    ds = sum(1 for x in wit[:-1] if is_der(x))
                    ecdsa += ds; v0_sigs += ds; uses_v0 = True; v0_scriptcode = len(ws) + 3
                    cms_upper += max(multisig_n(ws), ds)
                else:
                    ds = sum(1 for p in ps[:-1] if is_der(p))
                    ecdsa += ds; legacy_sigs += ds
                    cms_upper += max(multisig_n(redeem), ds)
            elif t == "witness_v0_keyhash":
                ecdsa += 1; v0_sigs += 1; cms_upper += 1; uses_v0 = True
                c["sha_c_hash160"] += 1; c["ripemd"] += 1
            elif t == "witness_v0_scripthash":
                ws = wit[-1] if wit else b""
                c["sha_c_wscript"] += sha_c(len(ws)); c["script_bytes"] += len(ws)
                ds = sum(1 for x in wit[:-1] if is_der(x))
                ecdsa += ds; v0_sigs += ds; uses_v0 = True; v0_scriptcode = len(ws) + 3
                cms_upper += max(multisig_n(ws), ds)
            elif t == "witness_v1_taproot":
                uses_v1 = True
                items = list(wit)
                if len(items) >= 2 and items[-1][:1] == b"\x50":
                    items = items[:-1]                                        # annex
                if len(items) == 1:
                    schnorr += 1; c["tr_keypath"] += 1
                elif len(items) >= 2:
                    c["tr_scriptpath"] += 1
                    control, tscript = items[-1], items[-2]
                    depth = max(0, (len(control) - 33) // 32)
                    c["sha_c_tapleaf"] += tagged_c(1 + varint_len(len(tscript)) + len(tscript))
                    c["sha_c_tapbranch"] += depth * tagged_c(64)
                    c["sha_c_taptweak"] += tagged_c(64)
                    c["tapscript_bytes"] += len(tscript); c["script_bytes"] += len(tscript)
                    c["ec_tweak_check"] += 1                                  # output key = internal + t*G check
                    schnorr += sum(1 for x in items[:-2] if len(x) in (64, 65))
            else:
                c[f"in_other_{t}"] += 1
            c["ecdsa"] += ecdsa; c["schnorr"] += schnorr
            c["ecdsa_cms_upper"] += cms_upper + 0
            # sighash preimage hashing
            c["sha_c_sighash_legacy"] += legacy_sigs * dsha_c(legacy_preimage)  # quadratic: whole tx per sig
            c["sha_c_sighash_v0"] += v0_sigs * dsha_c(156 + v0_scriptcode)
            c["sha_c_sighash_v1"] += schnorr * tagged_c(206)
            c["sha_c_bip340_chal"] += schnorr * tagged_c(96)
        if uses_v0:   # BIP143 per-tx midstate caches (Core: PrecomputedTransactionData)
            c["sha_c_sighash_v0"] += dsha_c(36 * nin) + dsha_c(4 * nin) + dsha_c(outs_ser)
        if uses_v1:   # BIP341 per-tx caches (single SHA)
            c["sha_c_sighash_v1"] += sha_c(36 * nin) + sha_c(8 * nin) + sha_c(spks_ser) + sha_c(4 * nin) + sha_c(outs_ser)
    c["sha_c_merkle"] = 3 * max(0, ntx - 1) * (2 if any_witness else 1)
    return dict(c)

def main():
    out, hfile = sys.argv[1], sys.argv[2]
    workers = int(sys.argv[3]) if len(sys.argv) > 3 else 3
    heights = [int(x) for x in open(hfile).read().split()]
    done = set()
    if os.path.exists(out):
        for line in open(out):
            done.add(json.loads(line)["height"])
    todo = [h for h in heights if h not in done]
    print(f"{len(done)} done, {len(todo)} to do", flush=True)
    q, res = queue.Queue(), queue.Queue()
    for h in todo:
        q.put(h)
    def work():
        rpc = RPC()
        while True:
            try:
                h = q.get_nowait()
            except queue.Empty:
                return
            try:
                res.put(scan_block(rpc, h))
            except Exception as e:
                res.put({"height": h, "error": repr(e)})
    for _ in range(workers):
        threading.Thread(target=work, daemon=True).start()
    with open(out, "a") as f:
        for n in range(len(todo)):
            f.write(json.dumps(res.get()) + "\n")
            if n % 100 == 0:
                f.flush(); print(f"{n}/{len(todo)}", flush=True)
    print("done", flush=True)

if __name__ == "__main__":
    main()
