#!/usr/bin/env python3
# Read-only. Signature batching across blocks: for the last N blocks, extracts each input's signing public key(s)
# (P2WPKH: witness[1]; P2PKH: the last scriptSig push; P2TR key path: the prevout's x-only key; P2SH-P2WPKH: witness[1];
# multisig and script paths are counted as "other") and reports, for windows of 1, 6, 36 and 144 blocks, the number of
# signatures, the number of DISTINCT keys, and so how much per-key work (decompression, table setup: ~14.5k cycles
# measured per key per chunk) a batch spanning the window would save. Writes /root/txproofs/keyreuse.json.
import base64, json, sys, time, urllib.request
ck = open('/root/.bitcoin/.cookie').read().strip()
hdr = {'Authorization': 'Basic ' + base64.b64encode(ck.encode()).decode(), 'Content-Type': 'application/json'}


def rpc(m, *p):
    b = json.dumps({'jsonrpc': '1.0', 'id': 1, 'method': m, 'params': list(p)}).encode()
    return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8332', b, hdr), timeout=600))['result']


def pushes(hexs):
    s, i, out = bytes.fromhex(hexs), 0, []
    while i < len(s):
        op = s[i]; i += 1
        if 1 <= op <= 75:
            out.append(s[i:i + op]); i += op
        elif op == 76:
            n = s[i]; out.append(s[i + 1:i + 1 + n]); i += 1 + n
        elif op == 77:
            n = int.from_bytes(s[i:i + 2], 'little'); out.append(s[i + 2:i + 2 + n]); i += 2 + n
        else:
            out.append(None)
    return out


N = int(sys.argv[1]) if len(sys.argv) > 1 else 144
tip = rpc('getblockcount')
blocks = []                # per block: list of key hex (one per single-key signature), count of "other" inputs
for h in range(tip - N + 1, tip + 1):
    b = rpc('getblock', rpc('getblockhash', h), 3)
    keys, other = [], 0
    for tx in b['tx'][1:]:
        for v in tx['vin']:
            t = v['prevout']['scriptPubKey']['type']
            wit = v.get('txinwitness', [])
            if t == 'witness_v0_keyhash' and len(wit) == 2:
                keys.append(wit[1])
            elif t == 'witness_v1_taproot' and len(wit) in (1, 2) and (len(wit) == 1 or wit[-1].startswith('50')):
                keys.append(v['prevout']['scriptPubKey']['hex'][4:])
            elif t == 'pubkeyhash':
                p = pushes(v['scriptSig']['hex'])
                if p and p[-1]:
                    keys.append(p[-1].hex())
                else:
                    other += 1
            elif t == 'scripthash' and len(wit) == 2 and len(wit[1]) == 66:
                keys.append(wit[1])
            else:
                other += 1
    blocks.append((h, keys, other))
    print(h, len(keys), other, flush=True)
res = {'tip': tip, 'blocks': N, 'windows': {}}
for W in (1, 6, 36, 144):
    if W > N:
        continue
    sigs = distinct = 0
    for s in range(0, N - W + 1, W):
        ks = [k for _, keys, _ in blocks[s:s + W] for k in keys]
        sigs += len(ks); distinct += len(set(ks))
    res['windows'][W] = {'single_key_sigs': sigs, 'distinct_keys': distinct, 'distinct_per_sig': round(distinct / sigs, 4)}
res['other_inputs'] = sum(o for _, _, o in blocks)
res['single_key_inputs'] = sum(len(k) for _, k, _ in blocks)
json.dump(res, open('/root/txproofs/keyreuse.json', 'w'), indent=1)
print(json.dumps(res, indent=1))
print('KEYREUSE_DONE')
