#!/usr/bin/env python3
# Read-only. How much of each new block could have been proved BEFORE the block arrived: polls the node's mempool every
# POLL seconds, remembers when each txid was first seen, and on each new block reports the share of its transactions
# (by count, by weight and by input count, a proxy for signature work) that had been in the mempool for at least 0, 30,
# 60, 120 and 300 s. Also counts mempool txs that left the mempool without being mined (replaced or evicted): proofs
# made for those would be wasted. The same for the node's own block template (getblocktemplate every TPL_EVERY s): a
# prover that pre-proves only what the template would mine wastes less. Writes one JSON line per block to OUT; runs
# until BLOCKS blocks are seen.
import base64, json, sys, time, urllib.request
ck = open('/root/.bitcoin/.cookie').read().strip()
hdr = {'Authorization': 'Basic ' + base64.b64encode(ck.encode()).decode(), 'Content-Type': 'application/json'}


def rpc(m, *p):
    b = json.dumps({'jsonrpc': '1.0', 'id': 1, 'method': m, 'params': list(p)}).encode()
    return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8332', b, hdr), timeout=600))['result']


POLL, BLOCKS, OUT = 5, int(sys.argv[1]) if len(sys.argv) > 1 else 8, '/root/txproofs/headstart.jsonl'
AGES = [0, 30, 60, 120, 300]
first = {}                 # txid -> first time seen in the mempool
in_tpl = {}                # txid -> first time seen in a block template
TPL_EVERY, last_tpl = 30, 0.0
mined = set()              # txids mined since the watch began
left = 0                   # txids that left the mempool unmined (counted when absent and not in a block)
tip = rpc('getblockcount')
start = time.time()
prev_pool = set()
seen_blocks = 0
warm = 600                 # the first block counted must arrive at least this long after start (mempool history)
while seen_blocks < BLOCKS:
    now = time.time()
    pool = set(rpc('getrawmempool'))
    for t in pool - prev_pool:
        first.setdefault(t, now)
    if now - last_tpl >= TPL_EVERY:
        last_tpl = now
        for t in rpc('getblocktemplate', {'rules': ['segwit']})['transactions']:
            in_tpl.setdefault(t['txid'], now)
    h = rpc('getblockcount')
    while h > tip:
        tip += 1
        b = rpc('getblock', rpc('getblockhash', tip), 2)
        rec = {'height': tip, 'since_start': round(now - start), 'txs': len(b['tx']) - 1, 'counted': now - start >= warm}
        tot = {'n': 0, 'w': 0, 'in': 0}
        ok = {a: {'n': 0, 'w': 0, 'in': 0} for a in AGES}
        okt = {a: {'n': 0, 'w': 0, 'in': 0} for a in AGES}
        for tx in b['tx'][1:]:
            mined.add(tx['txid'])
            w, ni = tx['weight'], len(tx['vin'])
            tot['n'] += 1; tot['w'] += w; tot['in'] += ni
            f = first.get(tx['txid'])
            for a in AGES:
                if f is not None and now - f >= a:
                    ok[a]['n'] += 1; ok[a]['w'] += w; ok[a]['in'] += ni
                g = in_tpl.get(tx['txid'])
                if g is not None and now - g >= a:
                    okt[a]['n'] += 1; okt[a]['w'] += w; okt[a]['in'] += ni
        rec['share'] = {a: {k: round(ok[a][k] / tot[k], 4) if tot[k] else None for k in tot} for a in AGES}
        rec['tpl_share'] = {a: {k: round(okt[a][k] / tot[k], 4) if tot[k] else None for k in tot} for a in AGES}
        rec['never_seen'] = {k: tot[k] - ok[0][k] for k in tot}
        if rec['counted']:
            seen_blocks += 1
        with open(OUT, 'a') as fh:
            fh.write(json.dumps(rec) + '\n')
        print(json.dumps(rec), flush=True)
    for t in prev_pool - pool:
        if t not in mined:
            left += 1
    prev_pool = pool
    with open(OUT + '.left', 'w') as fh:
        fh.write(json.dumps({'left_unmined': left, 'mined_seen': len(mined), 'tracked': len(first),
                             'tpl_tracked': len(in_tpl), 'tpl_unmined': len([t for t in in_tpl if t not in mined]),
                             'arrived_after_start': len([t for t, f0 in first.items() if f0 > start + 1]),
                             'elapsed': round(time.time() - start)}))
    time.sleep(POLL)
print('HEADSTART_DONE', flush=True)
