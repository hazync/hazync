import json,os,re,subprocess,struct
B=os.path.expanduser('~/dev/hazync-libzync/prover/block_962000.json'); H=os.path.expanduser('~/libzync-gpu/r18/host_r10m')
env=dict(os.environ,HAZYNC_PROFILE_LAYOUT='cost',HAZYNC_CHUNKS='16',HAZYNC_HINT_THREADS='2')
def t(n,edit,want):
    b=json.load(open(B)); edit(b); f=os.path.abspath(n+'.json'); json.dump(b,open(f,'w')); h=os.path.abspath(n+'.lzb')
    e=dict(env,HAZYNC_BLOCK=f); r0=subprocess.run([H,'libzync-hints',h],env=e,capture_output=True,text=True,errors='replace')
    r=subprocess.run([H,'agg-exec-fake'],env=dict(e,HAZYNC_LZ_HINTS=h),capture_output=True,text=True,errors='replace'); x=r.stdout+r.stderr
    for p in (f,h):
        if os.path.exists(p): os.remove(p)
    m=re.search(r'block INVALID (all_ok=\w+ pow=\w+ merkle=\w+ subsidy=\w+.*?bip30=\w+)',x,re.S); fl=' '.join(m.group(1).split()) if m else None
    print('PASS' if fl and f'{want}=false' in fl else 'WRONG',n,'hints rc',r0.returncode,'agg rc',r.returncode,'|',fl or [l[:150] for l in x.splitlines() if 'panicked' in l or 'Guest' in l or 'error' in l.lower()][-2:],flush=True)
def rich(b):
    x=bytearray.fromhex(b['coinbase_hex']); o=6; o+=1+36; L=x[o]; o+=1+L+4; o+=1; v=struct.unpack_from('<Q',x,o)[0]; struct.pack_into('<Q',x,o,v+5_000_000_000); b['coinbase_hex']=x.hex()
t('bad_nonce_pow',lambda b:b.update(nonce=b['nonce']^1),'pow')
t('coinbase_overpays',rich,'subsidy')
print('DONE')
