# resusage.py < cuobjdump --dump-resource-usage: REG/STACK/LOCAL of the eval_check and stepExec family, per arch
import re, sys
arch = None; rows = []
pat = re.compile(r"eval_check|rv32im_v2_\d+|poly_fp|stepExec|piece\d|_Zml8bb31|step_Top|exec_Top")
for line in sys.stdin:
    m = re.search(r"arch = (sm_\d+)", line)
    if m: arch = m.group(1); continue
    m = re.match(r"\s*Function (\S+):", line)
    if m: fn = m.group(1); continue
    m = re.search(r"REG:(\d+) STACK:(\d+) SHARED:(\d+) LOCAL:(\d+)", line)
    if m and pat.search(fn or ""):
        rows.append((arch, fn, *m.groups()))
seen = set()
for r in rows:
    if r in seen: continue
    seen.add(r)
    print("\t".join([r[0], r[2], r[3], r[5], r[1]]))
print(f"# arch\tREG\tSTACK\tLOCAL\tfunction ({len(seen)} rows)")
