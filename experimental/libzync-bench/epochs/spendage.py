#!/usr/bin/env python3
# Read-only. Spend age of every input in a sample of recent blocks: how many spends would cross an epoch boundary
# (a coin created in an earlier epoch than the block spending it) for several epoch lengths. Uses getblock verbosity 3
# (vin[].prevout.height). Writes /root/spendage/result.json, ends with SPENDAGE_DONE.
import base64, json, sys, time, urllib.request, collections
ck=open('/root/.bitcoin/.cookie').read().strip()
hdr={'Authorization':'Basic '+base64.b64encode(ck.encode()).decode(),'Content-Type':'application/json'}
def rpc(m,*p):
    b=json.dumps({'jsonrpc':'1.0','id':1,'method':m,'params':list(p)}).encode()
    return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8332',b,hdr),timeout=600))['result']
SPAN, N = 8064, 600
LS = [144, 1008, 2016, 4032]
tip = rpc('getblockcount')
heights = sorted({tip - SPAN + 1 + (i * SPAN) // N for i in range(N)})
age = collections.Counter()            # spend age (h - c) -> count
per_block = []                         # (h, inputs, outputs, {L: crossing spends})
t0 = time.time()
for n, h in enumerate(heights):
    b = rpc('getblock', rpc('getblockhash', h), 3)
    ins = 0; outs = 0; cross = {L: 0 for L in LS}
    for i, tx in enumerate(b['tx']):
        outs += len(tx['vout'])
        if i == 0:
            continue
        for v in tx['vin']:
            c = v['prevout']['height']
            ins += 1
            age[h - c] += 1
            for L in LS:
                if (c + L - 1) // L < (h + L - 1) // L:   # epoch of block x = ceil(x / L); boundary after block kL
                    cross[L] += 1
    per_block.append((h, ins, outs, cross))
    if n % 50 == 0:
        print(f'{n}/{len(heights)} h={h} {time.time()-t0:.0f}s', flush=True)
tot_in = sum(p[1] for p in per_block)
res = {'tip': tip, 'span': SPAN, 'blocks': len(per_block), 'inputs': tot_in, 'outputs': sum(p[2] for p in per_block),
       'crossing_fraction': {L: sum(p[3][L] for p in per_block) / tot_in for L in LS},
       'per_block': [[h, i, o, {str(L): c[L] for L in LS}] for h, i, o, c in per_block],
       'age_hist': sorted(age.items())}
json.dump(res, open('/root/spendage/result.json', 'w'))
print(json.dumps({k: res[k] for k in ('tip', 'blocks', 'inputs', 'outputs', 'crossing_fraction')}))
print('SPENDAGE_DONE', flush=True)
