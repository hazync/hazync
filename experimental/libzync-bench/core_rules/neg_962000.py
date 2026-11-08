import json,os,re,subprocess
B=os.path.expanduser('~/dev/hazync-libzync/prover/block_962000.json'); b0=open(B).read()
bounds=[tuple(map(int,m)) for m in re.findall(r'\[(\d+)\.\.(\d+)\)',open('r9m.chunks.log').read())]
base=json.loads(b0); cum=[];n=0
for t in base['txs']: cum.append(n); n+=len(t['prevouts'])
def run(name,ti,inp,edit,want):
    b=json.loads(b0); p=b['txs'][ti]['prevouts'][inp]; edit(p)
    f=f'neg_{name}.json'; json.dump(b,open(f,'w'))
    pos=cum[ti]+inp; c=[i for i,(l,h) in enumerate(bounds) if l<=pos<h][0]
    env=dict(os.environ,HAZYNC_PROFILE_LAYOUT='cost',HAZYNC_CHUNKS='16',HAZYNC_HINT_THREADS='2',HAZYNC_BLOCK=os.path.abspath(f),HAZYNC_LZ_HINTS=os.path.abspath('r9m.lzb'))
    r=subprocess.run([os.path.expanduser('~/libzync-gpu/r18/host_r9m'),'exec-chunk',str(c)],env=env,capture_output=True,text=True,errors='replace')
    m=re.search(r'all_valid=(\d)',r.stdout+r.stderr); got=m.group(1) if m else 'none rc=%d'%r.returncode
    print(f"{'PASS' if got==want else 'WRONG'} {name:22} chunk {c:2} all_valid={got} (want {want})",flush=True); os.remove(f)
def setp(**k): return lambda p:p.update(k)
run('maturity_ok',4,0,setp(coin_is_coinbase=1,coin_height=961000),'1')
run('maturity_immature',4,0,setp(coin_is_coinbase=1,coin_height=961950),'0')
run('height_lock_met',304,0,setp(coin_height=961999),'1')
run('height_lock_unmet',304,0,setp(coin_height=962000),'0')
v=base['txs'][4]['prevouts'][0]['value']
run('value_more',4,0,setp(value=v+1000),'1')
run('value_below_outputs',4,0,setp(value=0),'0')
print('NEG_DONE')
