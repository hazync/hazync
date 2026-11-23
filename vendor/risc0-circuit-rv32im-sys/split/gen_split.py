#!/usr/bin/env python3
"""gen_split.py: split risc0's generated rv32im `eval_check` call chain into several kernels.

Prover-only (hazync perf/eval-check-split). The circuit, METHOD_ID and the verifier are untouched: this
re-schedules the SAME arithmetic, and every output must be bit-identical to the stock kernel
(split/test_split_cpu.sh on CPU, HAZYNC_EVAL_CHECK=compare:<variant> on the GPU).

The zirgen output (kernels/cuda/eval_check_[0-3].cu, and its C++ twin kernels/cxx/rust_poly_fp_[0-3].cpp) is
one chain of 21 functions, poly_fp -> rv32im_v2_19 -> ... -> rv32im_v2_0, each ending in a tail call to the next.
Two scratch pools (Fp x33[1018], FpExt x34[125]) are declared in poly_fp and passed down by pointer; every
slot is written exactly once. The 20 callees are extern __device__ in other translation units, so each link of
the chain is a real ABI call.

This tool cuts the chain after the functions named in --cuts. Each piece becomes one __global__ kernel in a
single translation unit, its functions static __forceinline__ (no ABI frames inside a piece). What crosses a
cut goes through a global "carry" buffer (structure of arrays, one column per carried Fp, rows = a chunk of
the domain):
  * the by-value FpExt arguments of the call that was cut, and
  * every pool slot written in an earlier piece and read in a later one (reads of such a slot become loads
    from the carry; the writer also stores it). Carry columns are reused once their value is dead.
The last piece computes check[] exactly as the stock eval_check kernel does.

Options (⚠ see below): --inline-mul also #undefs __CUDACC_RDC__ before sppark's baby_bear.hpp is included, which makes
FpExt*FpExt and FpExt*Fp inline instead of __device__ __noinline__ calls (baby_bear.hpp:398). --keep-calls with
no --cuts reproduces the stock chain (separate __noinline__ functions) in one TU, to measure --inline-mul alone.
⚠ MEASURED 23.11.2026: a binary holding an --inline-mul TU failed the stock cudaMemcpyToSymbol(poly_mix) with
"invalid argument" (undefining __CUDACC_RDC__ evidently changes how that TU registers with the runtime), so
those variants are not built; inline FpExt multiplication needs a source rewrite, not the macro.

Usage:
  gen_split.py cuda <kernels/cuda dir> <out.cu> --prefix split --cuts 18,14,10,4 [--inline-mul] [--keep-calls]
  gen_split.py cxx  <kernels/cxx dir>  <out.cpp> --prefix split --cuts 18,14,10,4
  gen_split.py liveness <kernels/cuda dir>         # print the cut table
Re-run after any risc0 regeneration; the tool asserts the chain shape it relies on and refuses otherwise.
"""
import argparse, glob, heapq, os, re, sys

CHAIN = ["poly_fp"] + [f"rv32im_v2_{k}" for k in range(19, -1, -1)]


def die(msg):
    sys.exit(f"gen_split: {msg}")


def parse(dialect, d):
    files = sorted(glob.glob(os.path.join(d, "eval_check_[0-9].cu" if dialect == "cuda" else "rust_poly_fp_[0-9].cpp")))
    if len(files) != 4:
        die(f"expected 4 generated files in {d}, found {len(files)}")
    if dialect == "cuda":
        sig = re.compile(r'__device__ FpExt (\w+)\(uint32_t idx,\s*uint32_t size\s*,([^)]*)\)\s*\{')
    else:
        sig = re.compile(r'^FpExt (\w+)\(size_t cycle, size_t steps, FpExt\* poly_mix, ([^)]*)\) \{', re.M)
    fns = {}
    for f in files:
        t = open(f).read()
        for m in sig.finditer(t):
            name, params = m.group(1), m.group(2)
            i, depth = m.end(), 1
            while depth:
                c = t[i]
                depth += (c == '{') - (c == '}')
                i += 1
            ps = []
            for p in params.split(","):
                p = p.strip()
                pm = re.match(r'^(?:const )?(Fp\*\*|Fp\*|FpExt\*|FpExt|Fp) (\w+)$', p)
                if not pm:
                    die(f"{name}: unparsed parameter '{p}'")
                ps.append((pm.group(1), pm.group(2)))
            if name in fns:
                die(f"{name} defined twice")
            fns[name] = dict(params=ps, body=t[m.end():i - 1], file=os.path.basename(f))
    missing = [n for n in CHAIN if n not in fns]
    if missing or len(fns) != len(CHAIN):
        die(f"chain shape changed: missing {missing}, found {sorted(fns)}")
    return fns


def strip_comments(s):
    return re.sub(r'/\*.*?\*/', '', s).strip()


TAIL = re.compile(r'\n(\s*)auto (x\d+) = (rv32im_v2_\d+)\(([^;]*)\);\s*\n\s*return \2;\s*$')


def analyse(dialect, fns):
    """Symbolic walk of the chain: what each pointer parameter is, pool accesses, tail calls."""
    nlead = 2 if dialect == "cuda" else 3  # idx,size | cycle,steps,poly_mix
    if dialect == "cuda":
        env = {"ctrl": "G:ctrl", "out": "G:out", "data": "G:data", "mix": "G:mix", "accum": "G:accum",
               "x33": "PF", "x34": "PE"}
    else:
        env = {"args[0]": "G:accum", "args[1]": "G:data", "args[2]": "G:out", "args[3]": "G:mix",
               "x33": "PF", "x34": "PE"}
    info = {}
    for pos, n in enumerate(CHAIN):
        f = fns[n]
        body = f["body"]
        if n == "poly_fp":
            syms = dict(env)
            pf, pe = "x33", "x34"
        else:
            syms = f["sym_in"]
            pf = [p for p, s in syms.items() if s == "PF"]
            pe = [p for p, s in syms.items() if s == "PE"]
            if len(pf) > 1 or len(pe) > 1:
                die(f"{n}: more than one pool parameter")
            pf = pf[0] if pf else None
            pe = pe[0] if pe else None
        # pool accesses: (kind, slot, is_write, line_no)
        acc = []
        lines = body.split("\n")
        for ln, line in enumerate(lines):
            if re.match(r'^\s*(Fp|FpExt) x3[34]\[\d+\];\s*$', line):
                continue
            for kind, nm in (("F", pf), ("E", pe)):
                if not nm:
                    continue
                for m in re.finditer(rf'\b{nm}\[(\d+)\]', line):
                    is_w = bool(re.match(rf'^\s*{nm}\[{m.group(1)}\] = ', line)) and m.start() == line.index(nm + "[")
                    acc.append((kind, int(m.group(1)), is_w, ln))
        # globals and size maxima
        gmax = {}
        for p, s in syms.items():
            if not s.startswith("G:"):
                continue
            g = s[2:]
            pat = re.escape(p)
            for m in re.finditer(rf'(?<![\w\]]){pat}\[(\d+) \* (?:size|steps) \+', body):
                gmax[g] = max(gmax.get(g, -1), int(m.group(1)))
            for m in re.finditer(rf'(?<![\w\]]){pat}\[(\d+)\]', body):
                gmax[g + "#flat"] = max(gmax.get(g + "#flat", -1), int(m.group(1)))
        tail = None
        if n != "rv32im_v2_0":
            m = TAIL.search(body)
            if not m:
                die(f"{n}: no tail call found")
            callee = m.group(3)
            if callee != CHAIN[pos + 1]:
                die(f"{n}: tail-calls {callee}, expected {CHAIN[pos + 1]}")
            args = [strip_comments(a) for a in m.group(4).split(",")]
            lead = args[:nlead]
            want = ["idx", "size"] if dialect == "cuda" else ["cycle", "steps", "poly_mix"]
            if lead != want:
                die(f"{n}: tail call leads with {lead}")
            args = args[nlead:]
            cps = fns[callee]["params"]
            if len(args) != len(cps):
                die(f"{n}: {len(args)} args for {callee}'s {len(cps)} params")
            sym_out = {}
            for (ty, pn), a in zip(cps, args):
                s = syms.get(a, "V")
                if ty == "FpExt":
                    if s != "V" and not re.match(r'^(x|arg)\d+$', a):
                        die(f"{n}: odd FpExt arg {a}")
                    s = "V"
                elif ty == "FpExt*":
                    if s != "PE":
                        die(f"{n}: FpExt* arg {a} is {s}")
                elif ty == "Fp*":
                    if s not in ("PF",) and not s.startswith("G:"):
                        die(f"{n}: Fp* arg {a} is {s}")
                else:
                    die(f"{n}: unexpected param type {ty}")
                sym_out[pn] = s
            fns[callee]["sym_in"] = sym_out
            tail = dict(span=(m.start(), m.end()), indent=m.group(1), callee=callee, args=args)
        info[n] = dict(pf=pf, pe=pe, acc=acc, tail=tail, syms=syms, gmax=gmax)
    # pool slot table: writer position, reader positions; every slot written once, before its reads
    slots = {}
    for pos, n in enumerate(CHAIN):
        for kind, k, is_w, ln in info[n]["acc"]:
            s = slots.setdefault((kind, k), dict(w=None, r=[]))
            if is_w:
                if s["w"] is not None:
                    die(f"pool slot {kind}{k} written twice")
                s["w"] = (pos, ln)
            else:
                s["r"].append((pos, ln))
    for key, s in slots.items():
        if s["w"] is None:
            die(f"pool slot {key} read but never written")
        for r in s["r"]:
            if r <= s["w"]:
                die(f"pool slot {key} read before it is written")
    return info, slots


def segments_for(cuts):
    segs, cur = [], []
    cutset = set(cuts)
    for n in CHAIN:
        cur.append(n)
        if n in cutset:
            segs.append(cur)
            cur = []
    if cur:
        segs.append(cur)
    if [x for s in segs for x in s] != CHAIN:
        die("bad cut list")
    return segs


def plan(info, slots, segs, fns):
    seg_of = {n: i for i, s in enumerate(segs) for n in s}
    items = []  # (start_seg, end_seg, key, width)
    for (kind, k), s in slots.items():
        ws = seg_of[CHAIN[s["w"][0]]]
        rs = max(seg_of[CHAIN[p]] for p, _ in s["r"]) if s["r"] else ws
        if rs > ws:
            items.append((ws, rs, ("slot", kind, k), 1 if kind == "F" else 4))
    for i, s in enumerate(segs[:-1]):
        last = s[-1]
        t = info[last]["tail"]
        cps = fns[t["callee"]]["params"]
        for j, ((ty, pn), a) in enumerate(zip(cps, t["args"])):
            if ty == "FpExt":
                items.append((i, i + 1, ("cut", i, j), 4))
    # interval allocation of carry columns: a column is reused only by an item starting after the old one ended
    items.sort(key=lambda x: (x[0], x[1], str(x[2])))
    free, busy, ncol, alloc = [], [], 0, {}
    for st, en, key, w in items:
        while busy and busy[0][0] < st:
            _, c = heapq.heappop(busy)
            heapq.heappush(free, c)
        cols = []
        for _ in range(w):
            if free:
                cols.append(heapq.heappop(free))
            else:
                cols.append(ncol)
                ncol += 1
        for c in cols:
            heapq.heappush(busy, (en, c))
        alloc[key] = cols
    return seg_of, alloc, ncol, items


def ld(kind, cols):
    return f"SPLIT_LDF({cols[0]})" if kind == "F" else "SPLIT_LDE({},{},{},{})".format(*cols)


def emit(dialect, fns, info, slots, segs, prefix, inline_mul, keep_calls):
    seg_of, alloc, ncol, items = plan(info, slots, segs, fns)
    P = prefix
    out = []
    w = out.append
    w("// This file is GENERATED by vendor/risc0-circuit-rv32im-sys/split/gen_split.py from the zirgen output")
    w(f"// ({'eval_check_[0-3].cu' if dialect == 'cuda' else 'rust_poly_fp_[0-3].cpp'}). Do not edit; re-run the tool.")
    w(f"// prefix={P} cuts={[s[-1] for s in segs[:-1]]} pieces={len(segs)} carry_columns={ncol} "
      f"inline_mul={int(inline_mul)} keep_calls={int(keep_calls)}")
    for i, s in enumerate(segs):
        w(f"//   piece {i}: {' '.join(s)}")
    if dialect == "cuda":
        if inline_mul:
            w("// --inline-mul: sppark's baby_bear.hpp makes FpExt*FpExt and FpExt*Fp __noinline__ calls under -rdc;")
            w("// with the macro undefined here they are inline (same arithmetic: both call bb31_4_t::mul / a[i]*b).")
            w("#ifdef __CUDACC_RDC__")
            w("#undef __CUDACC_RDC__")
            w("#endif")
        w('#include "eval_check.cuh"')
        w('#include "cuda_util.h"')
        w('#include "supra/fp.h"')
        w("#include <cstdint>")
        w("#include <exception>")
        w("#include <cstring>")
        w("namespace risc0::circuit::rv32im_v2::cuda {")
        w(f"__constant__ Fp* {P}_carry;")
        w(f"__constant__ uint32_t {P}_stride;")
        w(f"#define SPLIT_ROW (idx & ({P}_stride - 1))")
        w(f"#define SPLIT_COL(c) {P}_carry[(size_t)(c) * {P}_stride + SPLIT_ROW]")
        w("#define SPLIT_LDF(c) (SPLIT_COL(c))")
        w("#define SPLIT_STF(c, v) (SPLIT_COL(c) = (v))")
        w("static __device__ __forceinline__ FpExt split_mk(Fp a, Fp b, Fp c, Fp d) { FpExt r; r[0] = a; r[1] = b; r[2] = c; r[3] = d; return r; }")
        w("#define SPLIT_LDE(a, b, c, d) split_mk(SPLIT_COL(a), SPLIT_COL(b), SPLIT_COL(c), SPLIT_COL(d))")
        w("#define SPLIT_STE(a, b, c, d, v) do { FpExt _t = (v); SPLIT_COL(a) = _t[0]; SPLIT_COL(b) = _t[1]; SPLIT_COL(c) = _t[2]; SPLIT_COL(d) = _t[3]; } while (0)")
        w("#define SPLIT_ZERO FpExt{0, 0, 0, 0}")
        qual = "static __device__ __noinline__" if keep_calls else "static __device__ __forceinline__"
        lead = "uint32_t idx, uint32_t size"
        lead_call = "idx, size"
    else:
        w('#include "fp.h"')
        w('#include "fpext.h"')
        w("#include <cstdint>")
        w("#include <cstddef>")
        w("constexpr size_t kInvRate = 4;")
        w("namespace risc0::circuit::rv32im_v2 {")
        w(f"Fp* {P}_carry;")
        w(f"size_t {P}_stride;")
        w(f"#define SPLIT_ROW (cycle & ({P}_stride - 1))")
        w(f"#define SPLIT_COL(c) {P}_carry[(size_t)(c) * {P}_stride + SPLIT_ROW]")
        w("#define SPLIT_LDF(c) (SPLIT_COL(c))")
        w("#define SPLIT_STF(c, v) (SPLIT_COL(c) = (v))")
        w("#define SPLIT_LDE(a, b, c, d) FpExt(SPLIT_COL(a), SPLIT_COL(b), SPLIT_COL(c), SPLIT_COL(d))")
        w("#define SPLIT_STE(a, b, c, d, v) do { FpExt _t = (v); SPLIT_COL(a) = _t.elems[0]; SPLIT_COL(b) = _t.elems[1]; SPLIT_COL(c) = _t.elems[2]; SPLIT_COL(d) = _t.elems[3]; } while (0)")
        w("#define SPLIT_ZERO FpExt(0)")
        qual = "static"
        lead = "size_t cycle, size_t steps, FpExt* poly_mix"
        lead_call = "cycle, steps, poly_mix"
    rename = re.compile(r'\b(rv32im_v2_\d+|poly_fp)\(')
    # declarations
    for n in CHAIN:
        ps = ", ".join(f"{'const ' if (dialect == 'cuda' and ty == 'Fp*' and info_sym(fns, info, n, pn).startswith('G:')) else ''}{ty} {pn}"
                       for ty, pn in fns[n]["params"])
        fns[n]["sig"] = f"FpExt {P}_{n}({lead}, {ps})"
        w(f"{qual} {fns[n]['sig']};")
    # bodies
    for pos, n in enumerate(CHAIN):
        f, inf = fns[n], info[n]
        s_here = seg_of[n]
        body = f["body"]
        # 1. the cut: replace the tail call by carry stores
        if inf["tail"] and seg_of[inf["tail"]["callee"]] != s_here:
            t = inf["tail"]
            ind = t["indent"].lstrip("\n")
            cps = fns[t["callee"]]["params"]
            st = []
            for j, ((ty, pn), a) in enumerate(zip(cps, t["args"])):
                if ty == "FpExt":
                    st.append(f"{ind}SPLIT_STE({','.join(map(str, alloc[('cut', s_here, j)]))}, {a});")
            body = body[:t["span"][0]] + "\n" + "\n".join(st) + f"\n{ind}return SPLIT_ZERO;\n" + body[t["span"][1]:]
        # 2. pool accesses
        lines = body.split("\n")
        pf, pe = inf["pf"], inf["pe"]
        nl = []
        for line in lines:
            if re.match(r'^\s*(Fp|FpExt) x3[34]\[\d+\];\s*$', line):
                nl.append(line)
                continue
            store = None
            for kind, nm in (("F", pf), ("E", pe)):
                if not nm:
                    continue
                wm = re.match(rf'^(\s*){nm}\[(\d+)\] = ', line)
                if wm:
                    key = ("slot", kind, int(wm.group(2)))
                    if key in alloc:
                        store = (wm.group(1), kind, alloc[key], f"{nm}[{wm.group(2)}]")

                def rd(m, kind=kind, nm=nm):
                    k = int(m.group(1))
                    s = slots[(kind, k)]
                    wseg = seg_of[CHAIN[s["w"][0]]]
                    if wseg < s_here:
                        return ld(kind, alloc[("slot", kind, k)])
                    return m.group(0)
                if wm:
                    head, rest = line[:wm.end()], line[wm.end():]
                    line = head + re.sub(rf'\b{nm}\[(\d+)\]', rd, rest)
                else:
                    line = re.sub(rf'\b{nm}\[(\d+)\]', rd, line)
            nl.append(line)
            if store:
                ind, kind, cols, ref = store
                if kind == "F":
                    nl.append(f"{ind}SPLIT_STF({cols[0]}, {ref});")
                else:
                    nl.append(f"{ind}SPLIT_STE({','.join(map(str, cols))}, {ref});")
        body = "\n".join(nl)
        body = rename.sub(lambda m: f"{P}_{m.group(1)}(", body)
        w(f"{qual} {f['sig']} {{{body}}}")
    # piece entry points
    gexpr_cuda = {"G:ctrl": "ctrl", "G:out": "out", "G:data": "data", "G:mix": "mix", "G:accum": "accum"}
    gexpr_cxx = {"G:accum": "args[0]", "G:data": "args[1]", "G:out": "args[2]", "G:mix": "args[3]"}
    for i, s in enumerate(segs):
        first = s[0]
        if first == "poly_fp":
            call = (f"{P}_poly_fp(idx, size, ctrl, out, data, mix, accum)" if dialect == "cuda"
                    else f"{P}_poly_fp(cycle, steps, poly_mix, args)")
        else:
            prev = segs[i - 1][-1]
            t = info[prev]["tail"]
            cargs = []
            for j, (ty, pn) in enumerate(fns[first]["params"]):
                sym = fns[first]["sym_in"][pn]
                if sym == "PF":
                    cargs.append("x33")
                elif sym == "PE":
                    cargs.append("x34")
                elif sym.startswith("G:"):
                    cargs.append((gexpr_cuda if dialect == "cuda" else gexpr_cxx)[sym])
                else:
                    cargs.append(ld("E", alloc[("cut", i - 1, j)]))
            call = f"{P}_{first}({lead_call}, {', '.join(cargs)})"
        last = i == len(segs) - 1
        if dialect == "cuda":
            w(f"__global__ void {P}_piece{i}(Fp* check, const Fp* ctrl, const Fp* data, const Fp* accum, const Fp* mix,")
            w("    const Fp* out, const Fp rou, uint32_t po2, uint32_t domain, uint32_t base, uint32_t rows) {")
            w("  uint32_t t = blockDim.x * blockIdx.x + threadIdx.x;")
            w("  if (t >= rows) return;")
            w("  uint32_t idx = base + t;")
            w("  uint32_t size = domain;")
            if first != "poly_fp":
                w("  Fp x33[1018];")
                w("  FpExt x34[125];")
            w(f"  FpExt tot = {call};")
            if last:
                w("  uint32_t cycle = idx;")
                w("  Fp x = pow(rou, cycle);")
                w("  Fp y = pow(Fp(Fp(3) * x), 1 << po2);  // explicit Fp: NVRTC sees pow(float/double,int) overloads too")
                w("  FpExt ret = tot * inv(y - Fp(1));")
                for k in range(4):
                    w(f"  check[domain * {k} + cycle] = ret[{k}];")
            else:
                w("  (void)tot;")
            w("}")
        else:
            w(f"FpExt {P}_piece{i}(size_t cycle, size_t steps, FpExt* poly_mix, Fp** args) {{")
            if first != "poly_fp":
                w("  Fp x33[1018];")
                w("  FpExt x34[125];")
            w(f"  return {call};")
            w("}")
    if dialect == "cuda":
        w(f"constexpr uint32_t {P}_kPieces = {len(segs)};")
        w(f"constexpr uint32_t {P}_kCarryCols = {ncol};")
        w("}  // namespace risc0::circuit::rv32im_v2::cuda")
        w("using namespace risc0::circuit::rv32im_v2::cuda;")
        w(f"""
// Host launcher. Runs every piece over one chunk of rows, then the next chunk; the carry holds one chunk.
// carry_bytes caps the carry buffer (the chunk is the largest power of two that fits, at least 1024 rows).
extern "C" const char* {P}_eval_check(Fp* check, const Fp* ctrl, const Fp* data, const Fp* accum,
                                      const Fp* mix, const Fp* out, const Fp& rou, uint32_t po2,
                                      uint32_t domain, size_t carry_bytes) {{
  try {{
    static Fp* carry = nullptr;
    static size_t carry_have = 0;
    uint32_t chunk = domain;
    while (chunk > 1024 && (size_t)chunk * {P}_kCarryCols * sizeof(Fp) > carry_bytes) chunk >>= 1;
    size_t need = (size_t)chunk * ({P}_kCarryCols ? {P}_kCarryCols : 1) * sizeof(Fp);
    if (need > carry_have) {{
      if (carry) CUDA_OK(cudaFree(carry));
      carry = nullptr; carry_have = 0;
      CUDA_OK(cudaMalloc(&carry, need));
      carry_have = need;
    }}
    CudaStream stream;
    CUDA_OK(cudaMemcpyToSymbolAsync({P}_carry, &carry, sizeof(carry), 0, cudaMemcpyHostToDevice, stream));
    CUDA_OK(cudaMemcpyToSymbolAsync({P}_stride, &chunk, sizeof(chunk), 0, cudaMemcpyHostToDevice, stream));
    auto cfg = getSimpleConfig(chunk);
    for (uint32_t base = 0; base < domain; base += chunk) {{""")
        for i in range(len(segs)):
            w(f"      {P}_piece{i}<<<cfg.grid, cfg.block, 0, stream>>>(check, ctrl, data, accum, mix, out, rou, po2, domain, base, chunk);")
        w(f"""    }}
    CUDA_OK(cudaPeekAtLastError());
    CUDA_OK(cudaStreamSynchronize(stream));
  }} catch (const std::exception& err) {{
    return strdup(err.what());
  }} catch (...) {{
    return strdup("Generic exception");
  }}
  return nullptr;
}}
extern "C" uint32_t {P}_carry_cols() {{ return {P}_kCarryCols; }}""")
    else:
        w(f"constexpr size_t {P}_kPieces = {len(segs)};")
        w(f"constexpr size_t {P}_kCarryCols = {ncol};")
        w(f"typedef FpExt (*{P}_piece_fn)(size_t, size_t, FpExt*, Fp**);")
        w(f"{P}_piece_fn {P}_pieces[] = {{" + ", ".join(f"{P}_piece{i}" for i in range(len(segs))) + "};")
        w(f"size_t {P}_npieces = {len(segs)};")
        w(f"size_t {P}_ncols = {ncol};")
        w("}  // namespace risc0::circuit::rv32im_v2")
    # gmax summary as a comment (buffer extents the generated code reads)
    gm = {}
    for n in CHAIN:
        for k, v in info[n]["gmax"].items():
            gm[k] = max(gm.get(k, -1), v)
    w(f"// buffer extents read (max column / flat index): {sorted(gm.items())}")
    return "\n".join(out) + "\n", ncol, gm


def info_sym(fns, info, n, pn):
    if n == "poly_fp":
        return "G:" + pn if pn in ("ctrl", "out", "data", "mix", "accum") else "?"
    return fns[n]["sym_in"].get(pn, "V")


def liveness(info, slots):
    print("cut after        live Fp  live FpExt  by-value FpExt args to callee")
    for i in range(len(CHAIN) - 1):
        lf = sum(1 for (k, _), s in slots.items() if k == "F" and s["w"][0] <= i and any(p > i for p, _ in s["r"]))
        le = sum(1 for (k, _), s in slots.items() if k == "E" and s["w"][0] <= i and any(p > i for p, _ in s["r"]))
        t = info[CHAIN[i]]["tail"]
        nv = sum(1 for a in t["args"] if not a.startswith("x3") and a not in info[CHAIN[i]]["syms"]) if t else 0
        print(f"{CHAIN[i]:16s} {lf:7d} {le:10d}  {nv}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dialect", choices=["cuda", "cxx", "liveness"])
    ap.add_argument("src")
    ap.add_argument("out", nargs="?")
    ap.add_argument("--prefix", default="split")
    ap.add_argument("--cuts", default="18,14,10,4", help="cut after these rv32im_v2_N (or poly_fp); '' = none")
    ap.add_argument("--inline-mul", action="store_true")
    ap.add_argument("--keep-calls", action="store_true")
    a = ap.parse_args()
    dialect = "cuda" if a.dialect == "liveness" else a.dialect
    fns = parse(dialect, a.src)
    info, slots = analyse(dialect, fns)
    if a.dialect == "liveness":
        liveness(info, slots)
        return
    cuts = [c if c == "poly_fp" else f"rv32im_v2_{c}" for c in a.cuts.split(",") if c.strip()]
    if a.keep_calls and cuts:
        die("--keep-calls is for the uncut chain only")
    if a.inline_mul and dialect != "cuda":
        die("--inline-mul is a CUDA-only option")
    segs = segments_for(cuts)
    src, ncol, gm = emit(dialect, fns, info, slots, segs, a.prefix, a.inline_mul, a.keep_calls)
    open(a.out, "w").write(src)
    print(f"gen_split: wrote {a.out}: {len(segs)} pieces, {ncol} carry columns ({4 * ncol} B/row), extents {gm}")


if __name__ == "__main__":
    main()
