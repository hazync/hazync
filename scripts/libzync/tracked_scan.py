#!/usr/bin/env python3
# tracked_scan.py [tracked.json]: the list reproduce/libzync-tracked.json, from a full node. Read-only on the node.
# With a list given, it continues from that list's tip. Run it before proving past the list's tip: a block spending only
# tracked coins creates new tracked coins, and mkblock.py refuses a height the list does not reach.
# How many transactions could ever repeat? A tx can repeat only if EVERY input is re-created, so it is in the
# cone only if all its inputs are. Roots: pre-BIP34 coinbases indicating a future height Y. A tx's level is the largest Y
# among its inputs: it cannot repeat before that height. Reports the cone per horizon. The parser must consume each block
# exactly and match its tx count, or the run aborts.
import base64, hashlib, json, struct, sys, time, urllib.request, collections
ck=open('/root/.bitcoin/.cookie').read().strip()
hdr={'Authorization':'Basic '+base64.b64encode(ck.encode()).decode(),'Content-Type':'application/json'}
def rpc(m,*p):
    b=json.dumps({'jsonrpc':'1.0','id':1,'method':m,'params':list(p)}).encode()
    return json.load(urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:8332',b,hdr),timeout=300))['result']
def raw(hh): return bytes.fromhex(rpc('getblock',hh,0))
def vi(b,o):
    c=b[o]
    if c<0xfd: return c,o+1
    if c==0xfd: return b[o+1]|b[o+2]<<8,o+3
    if c==0xfe: return struct.unpack_from('<I',b,o+1)[0],o+5
    return struct.unpack_from('<Q',b,o+1)[0],o+9
def indicated(s):
    if not s: return None
    op=s[0]
    if 0x51<=op<=0x60: return op-0x50
    if 1<=op<=5 and len(s)>op:
        d=s[1:1+op]; v=int.from_bytes(d,'little')
        if d[-1]&0x80 or v==0 or (op>1 and d[-1]==0 and not d[-2]&0x80): return None
        if op==1 and v<=16: return None   # CScript() << v is OP_v for 1..16, so a 1-byte push of it is not BIP34 (guest hz_indicated_height)
        return v
    return None
HORIZON=100_000_000; BIP34=227931
live={}                 # (txid,vout) -> level
tracked_cb={}           # txid of every tracked coinbase -> level (a repeat of it is tracked)
ALL=[]                  # every tracked output: txid (display hex), vout, created height, level, is coinbase
stats=collections.Counter(); inputs_spent=collections.Counter(); byb=collections.Counter()
tip=rpc('getblockcount'); t0=time.time(); start=1
if len(sys.argv) > 1:
    old=json.load(open(sys.argv[1])); assert old['horizon']==HORIZON
    ALL=old['outputs']; start=old['tip']+1
    tracked_cb={bytes.fromhex(a[0])[::-1]:a[3] for a in ALL if a[4]}
    live={(bytes.fromhex(t)[::-1],struct.pack('<I',v)):next(a[3] for a in ALL if a[0]==t and a[1]==v) for t,v in old['live']}
for h in range(start,tip+1):
    b=raw(rpc('getblockhash',h)); n,o=vi(b,80)
    for ti in range(n):
        s=o; o+=4; seg=b[o]==0 and b[o+1]==1
        if seg: o+=2
        vin,o=vi(b,o); bs=o; levels=[]; allin=True
        for _ in range(vin):
            k=b[o:o+36]; o+=36; L,o=vi(b,o)
            if ti==0: cs=b[o:o+L]
            o+=L+4
            if ti: 
                lv=live.pop((k[:32],k[32:]),None) if live else None
                if lv is None: allin=False
                else: levels.append(lv); inputs_spent[0]+=1
        vout,o=vi(b,o); vs=o
        for _ in range(vout):
            o+=8; L,o=vi(b,o); o+=L
        ve=o
        if seg:
            for _ in range(vin):
                c,o=vi(b,o)
                for _ in range(c):
                    L,o=vi(b,o); o+=L
        o+=4
        lvl=None
        pre=(b[s:s+4]+b[(s+6 if seg else s+4):ve]+b[o-4:o])
        txid=hashlib.sha256(hashlib.sha256(pre).digest()).digest()
        if ti==0:
            if h<BIP34:
                y=indicated(cs)
                if y and h<y<HORIZON: lvl=y
            # the repeat of a tracked coinbase is tracked too (guest agg_lite.rs, coinbase_prior == REPEATABLE)
            if lvl is None and txid in tracked_cb: lvl=tracked_cb[txid]
            if lvl is not None: tracked_cb[txid]=lvl
        elif allin and levels: lvl=max(levels)
        if lvl is not None:
            p=vs; 
            for i in range(vout):
                p+=8; L,p=vi(b,p); p+=L
                live[(txid,struct.pack('<I',i))]=lvl
                ALL.append([txid[::-1].hex(),i,h,lvl,int(ti==0)])
            bk=lvl if lvl<5_000_000 else 5_000_000 if lvl<20_000_000 else 20_000_000
            stats[bk]+=1; byb[h//100000]+=1
    assert o==len(b),('parse',h,o,len(b))
    if h%20000==0:
        print(h,int(time.time()-t0),'s live',len(live),'cone tx',sum(stats.values()),'inputs',inputs_spent[0],sorted(stats.items())[:6],flush=True)
print('CONE by level',sorted(stats.items())); print('per 100k blocks',sorted(byb.items())); print('live at tip',len(live),'cone spends',inputs_spent[0]); json.dump({'tip':tip,'horizon':HORIZON,'outputs':ALL,'live':[[k[0][::-1].hex(),struct.unpack('<I',k[1])[0]] for k in live]},open(sys.argv[1] if len(sys.argv)>1 else 'tracked.json','w'),indent=0)
print('CONE_DONE')
