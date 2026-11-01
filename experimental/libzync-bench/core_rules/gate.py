import os,re,subprocess
R=os.path.expanduser('~/libzync-gpu/r18'); O=R+'/r12'; H=R+'/host_r12m'
def go(name,B,chunks,baseline=None):
    env=dict(os.environ,HAZYNC_PROFILE_LAYOUT='cost',HAZYNC_CHUNKS=str(chunks),HAZYNC_HINT_THREADS='2',HAZYNC_BLOCK=B)
    subprocess.run([H,'libzync-hints',f'{O}/{name}.lzb'],env=env,capture_output=True); e=dict(env,HAZYNC_LZ_HINTS=f'{O}/{name}.lzb')
    if baseline:
        j=[re.search(r'all_valid=(\d) journal sha256 (\w+)',subprocess.run([H,'exec-chunk',str(i)],env=e,capture_output=True,text=True,errors='replace').stdout).groups() for i in range(chunks)]
        old=re.findall(r'all_valid=(\d) journal sha256 (\w+)',open(baseline).read())
        print('CHUNK_JOURNALS vs round 8:','IDENTICAL' if j==old else 'DIFFER',len(j),flush=True)
    t=subprocess.run([H,'agg-exec-fake'],env=e,capture_output=True,text=True,errors='replace'); x=t.stdout+t.stderr
    c=sum(map(int,re.findall(r'executed: (\d+) cycles',x))); a=re.search(r'block validation cycles\s+(\d+)',x)
    bad=re.findall(r'block INVALID[^\n]*|panicked[^\n]*',x)[:2]
    print(name,'rc',t.returncode,'chunks',f'{c:,}','aggregate',a and f'{int(a.group(1)):,}','whole',a and f'{c+int(a.group(1)):,}',bad,flush=True)
go('b962000',os.path.expanduser('~/dev/hazync-libzync/prover/block_962000.json'),16,R+'/r9/main.chunks.log')
go('b170060',R+'/exc/block_170060.json',2)
go('b692261',R+'/exc/block_692261.json',16)
