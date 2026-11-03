#!/usr/bin/env python3
# Read-only. Phase A's waste (README §7): a prover that pre-proves the node's block template for height tip+1 wastes
# the template transactions the next block does not mine (they are re-proved for the height after). Polls
# getblocktemplate every POLL s; on each new block compares the LAST template seen before it with the block.
# Per block, by input count (signature work): template inputs mined (useful), template inputs NOT mined (waste),
# block inputs NOT in the template (the block-time chunk). One JSON line per block to OUT; runs for BLOCKS blocks.
# Runs on this machine; every call is a read-only bitcoin-cli on the bridge node over ssh.
import json, subprocess, sys, time


def rpc(m, *p):
    args = ['ssh', 'hazync-coord', 'bitcoin-cli', m] + [json.dumps(x) if not isinstance(x, (str, int)) else str(x) for x in p]
    if m == 'getblocktemplate':
        args[-1] = "'" + args[-1] + "'"
    out = subprocess.run(args, capture_output=True, text=True, timeout=600, check=True).stdout
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return out.strip()


def n_inputs(hexs):
    b = bytes.fromhex(hexs)
    o = 4
    if b[4] == 0 and b[5] == 1:
        o += 2
    f = b[o]
    return f if f < 0xfd else int.from_bytes(b[o + 1:o + 1 + {0xfd: 2, 0xfe: 4, 0xff: 8}[f]], 'little')


POLL, BLOCKS, OUT = 15, int(sys.argv[1]) if len(sys.argv) > 1 else 8, 'template_waste.jsonl'
tip = rpc('getblockcount')
tpl = None
seen = 0
while seen < BLOCKS:
    t = rpc('getblocktemplate', {'rules': ['segwit']})
    if t['height'] == tip + 1:
        tpl = {x['txid']: n_inputs(x['data']) for x in t['transactions']}
    h = rpc('getblockcount')
    if h > tip:
        for hh in range(tip + 1, h + 1):
            if hh == tip + 1 and tpl is not None:
                blk = rpc('getblock', rpc('getblockhash', hh), 1)
                mined = set(blk['tx'][1:])
                useful = sum(n for k, n in tpl.items() if k in mined)
                waste = sum(n for k, n in tpl.items() if k not in mined)
                rec = {'height': hh, 'template_txs': len(tpl), 'block_txs': len(mined),
                       'template_inputs': sum(tpl.values()), 'useful_inputs': useful, 'waste_inputs': waste,
                       'block_txs_not_in_template': len(mined - set(tpl)), 'time': int(time.time())}
                print(json.dumps(rec), flush=True)
                open(OUT, 'a').write(json.dumps(rec) + '\n')
                seen += 1
        tip, tpl = h, None
        continue
    time.sleep(POLL)
