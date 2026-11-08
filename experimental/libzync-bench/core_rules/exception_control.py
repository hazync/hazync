import json,os,re,subprocess
R=os.path.expanduser('~/libzync-gpu/r18'); O=R+'/r12'; H=R+'/host_r12m'
def go(n,b0,chunks):
    b=json.load(open(b0)); b['nonce']^=1; f=f'{O}/{n}.json'; json.dump(b,open(f,'w'))
    env=dict(os.environ,HAZYNC_PROFILE_LAYOUT='cost',HAZYNC_CHUNKS=str(chunks),HAZYNC_HINT_THREADS='2',HAZYNC_BLOCK=f)
    r=subprocess.run([H,'libzync-hints',f'{O}/{n}.lzb'],env=env,capture_output=True,text=True,errors='replace')
    e=dict(env,HAZYNC_LZ_HINTS=f'{O}/{n}.lzb'); v=[]
    for i in range(chunks):
        t=subprocess.run([H,'exec-chunk',str(i)],env=e,capture_output=True,text=True,errors='replace')
        m=re.search(r'all_valid=(\d)',t.stdout+t.stderr); v.append(m.group(1) if m else 'x')
    print(n,'hints rc',r.returncode,'chunks all_valid',''.join(v),flush=True); os.remove(f)
go('ctl170060',R+'/exc/block_170060.json',2)
go('ctl692261',R+'/exc/block_692261.json',16)
