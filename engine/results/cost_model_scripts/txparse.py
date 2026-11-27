# minimal segwit tx parser: per input, scriptSig bytes, witness items' lengths, and the executed script:
# tapscript leaf (P2TR script path), P2WSH witness script, P2SH redeem script (and its witness script when wrapped)
import json, sys
def varint(b, i):
    x = b[i]
    if x < 0xfd: return x, i + 1
    n = {0xfd: 2, 0xfe: 4, 0xff: 8}[x]
    return int.from_bytes(b[i+1:i+1+n], 'little'), i + 1 + n
def parse(raw):
    b = bytes.fromhex(raw); i = 4; seg = False
    if b[i] == 0 and b[i+1] == 1: seg = True; i += 2
    n, i = varint(b, i); ins = []
    for _ in range(n):
        i += 36; l, i = varint(b, i); ss = b[i:i+l]; i += l + 4; ins.append({'ss': ss, 'wit': []})
    m, i = varint(b, i)
    for _ in range(m):
        i += 8; l, i = varint(b, i); i += l
    if seg:
        for inp in ins:
            k, i = varint(b, i)
            for _ in range(k):
                l, i = varint(b, i); inp['wit'].append(b[i:i+l]); i += l
    return ins
def last_push(ss):
    i = 0; last = b''
    while i < len(ss):
        op = ss[i]; i += 1
        if 1 <= op <= 75: last = ss[i:i+op]; i += op
        elif op == 76: l = ss[i]; last = ss[i+1:i+1+l]; i += 1 + l
        elif op == 77: l = int.from_bytes(ss[i:i+2], 'little'); last = ss[i+2:i+2+l]; i += 2 + l
        elif op == 78: l = int.from_bytes(ss[i:i+4], 'little'); last = ss[i+4:i+4+l]; i += 4 + l
        else: last = b''
    return last
def executed(spk_hex, inp):
    spk = bytes.fromhex(spk_hex); w = inp['wit']
    if len(spk) == 34 and spk[0] == 0x51 and spk[1] == 0x20:   # P2TR
        n = len(w)
        if n >= 2 and w[-1][:1] == b'\x50': n -= 1
        return ('tapscript', len(w[n-2])) if n >= 2 else ('p2tr-key', 0)
    if len(spk) == 34 and spk[0] == 0 and spk[1] == 0x20: return ('p2wsh', len(w[-1]) if w else 0)
    if len(spk) == 22 and spk[0] == 0 and spk[1] == 0x14: return ('p2wpkh', 0)
    if len(spk) == 23 and spk[0] == 0xa9:
        rs = last_push(inp['ss'])
        if len(rs) == 34 and rs[0] == 0 and rs[1] == 0x20: return ('p2sh-p2wsh', len(w[-1]) if w else 0)
        if len(rs) == 22 and rs[0] == 0 and rs[1] == 0x14: return ('p2sh-p2wpkh', 0)
        return ('p2sh', len(rs))
    return ('bare', 0)
if __name__ == '__main__':
    d = json.load(open(sys.argv[1])); kinds = {}
    for t in d['txs']:
        for inp, pv in zip(parse(t['raw']), t['prevouts']):
            k, l = executed(pv['spk'], inp); s = kinds.setdefault(k, [0, 0, 0]); s[0] += 1; s[1] += l; s[2] = max(s[2], l)
    for k, (n, tot, mx) in sorted(kinds.items()): print(f"{k:12} inputs {n:6}  script bytes {tot:10}  max {mx}")
