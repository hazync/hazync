import sys, collections
syms = open(sys.argv[2], "rb").read()
def name(o):
    try:
        o = int(o); e = syms.index(b"\0", o); return syms[o:e].decode(errors="replace")
    except Exception: return o
stacks = collections.Counter()
for l in open(sys.argv[1]):
    sp, v = l.rstrip("\n").rsplit(" ", 1); stacks[sp] += int(v)
total = sum(stacks.values()); selfc, incl = collections.Counter(), collections.Counter()
for sp, v in stacks.items():
    fs = sp.split(";"); selfc[fs[-1]] += v
    for f in set(fs): incl[f] += v
top = int(sys.argv[3]) if len(sys.argv) > 3 else 50
print(f"total {total:,}")
print("SELF"); [print(f"{100*v/total:6.2f} {100*incl[f]/total:6.2f}  {name(f)[:150]}") for f, v in selfc.most_common(top)]
print("INCLUSIVE"); [print(f"{100*v/total:6.2f}  {name(f)[:150]}") for f, v in incl.most_common(top)]
