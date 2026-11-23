// test_split_cpu.cpp: the split eval_check chain must give BIT-IDENTICAL results to the stock zirgen code.
//
// Built by split/test_split_cpu.sh against the stock C++ twin (kernels/cxx/rust_poly_fp_[0-3].cpp, function
// poly_fp) and the split C++ that gen_split.py emits from the same files (split_pieces[]). For every row it
// runs stock poly_fp, then the pieces in order (chunk by chunk, with the carry buffer between pieces exactly as
// the CUDA launcher does it), and compares the four raw Montgomery words of the FpExt result.
//
//   test_split_cpu random <log2 rows> <seed>    random field elements in every input buffer
//   test_split_cpu dump <dir> [chunk_log2]      real inputs written by the CUDA host with HAZYNC_EVAL_CHECK_DUMP
//                                               (one eval_check call of a real segment); also recomputes check[]
//                                               from stock poly_fp and compares it with the GPU's stock check[].
// Exit 0 only if every row matches.
#include "fp.h"
#include "fpext.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include <string>
#include <vector>

namespace risc0::circuit::rv32im_v2 {
FpExt poly_fp(size_t cycle, size_t steps, FpExt* poly_mix, Fp** args);
extern Fp* split_carry;
extern size_t split_stride;
typedef FpExt (*split_piece_fn)(size_t, size_t, FpExt*, Fp**);
extern split_piece_fn split_pieces[];
extern size_t split_npieces;
extern size_t split_ncols;
} // namespace risc0::circuit::rv32im_v2

using namespace risc0;
using namespace risc0::circuit::rv32im_v2;

// extents the generated code reads (gen_split.py prints them): data 211 cols, accum 103 cols, out 91, mix 36
static const size_t kData = 211, kAccum = 103, kOut = 91, kMix = 36, kPolyMix = 484;

static bool read_file(const std::string& p, void* dst, size_t n) {
  FILE* f = fopen(p.c_str(), "rb");
  if (!f) { fprintf(stderr, "cannot open %s\n", p.c_str()); return false; }
  size_t got = fread(dst, 1, n, f);
  int extra = fgetc(f);
  fclose(f);
  if (got != n || extra != EOF) { fprintf(stderr, "%s: size mismatch (want %zu bytes)\n", p.c_str(), n); return false; }
  return true;
}

static bool same(const FpExt& a, const FpExt& b) {
  for (int i = 0; i < 4; i++)
    if (a.elems[i].asRaw() != b.elems[i].asRaw()) return false;
  return true;
}

int main(int argc, char** argv) {
  if (argc < 3) { fprintf(stderr, "usage: %s random <log2 rows> <seed> | dump <dir> [chunk_log2]\n", argv[0]); return 2; }
  std::string mode = argv[1];
  size_t domain = 0, po2 = 0;
  uint32_t rou_raw = 0;
  std::vector<Fp> data, accum, out, mix, gpu_check;
  std::vector<FpExt> pmix(kPolyMix);
  size_t chunk_log2 = 10;
  bool have_check = false;
  if (mode == "random") {
    domain = size_t(1) << atoi(argv[2]);
    std::mt19937_64 rng(argc > 3 ? strtoull(argv[3], nullptr, 10) : 1);
    auto rnd = [&]() { return Fp(uint32_t(rng() % Fp::P)); };
    data.resize(kData * domain); accum.resize(kAccum * domain); out.resize(kOut); mix.resize(kMix);
    for (auto& v : data) v = rnd();
    for (auto& v : accum) v = rnd();
    for (auto& v : out) v = rnd();
    for (auto& v : mix) v = rnd();
    for (auto& v : pmix) v = FpExt(rnd(), rnd(), rnd(), rnd());
    chunk_log2 = std::min<size_t>(chunk_log2, atoi(argv[2]) - 2);
  } else if (mode == "dump") {
    std::string d = argv[2];
    FILE* m = fopen((d + "/meta.txt").c_str(), "r");
    if (!m) { fprintf(stderr, "no meta.txt in %s\n", d.c_str()); return 2; }
    unsigned long long dd, pp; unsigned rr;
    if (fscanf(m, "domain %llu po2 %llu rou_raw %u", &dd, &pp, &rr) != 3) { fprintf(stderr, "bad meta.txt\n"); return 2; }
    fclose(m);
    domain = dd; po2 = pp; rou_raw = rr;
    data.resize(kData * domain); accum.resize(kAccum * domain); out.resize(kOut); mix.resize(kMix);
    gpu_check.resize(4 * domain);
    if (!read_file(d + "/data.bin", data.data(), data.size() * 4) ||
        !read_file(d + "/accum.bin", accum.data(), accum.size() * 4) ||
        !read_file(d + "/out.bin", out.data(), out.size() * 4) ||
        !read_file(d + "/mix.bin", mix.data(), mix.size() * 4) ||
        !read_file(d + "/poly_mix.bin", pmix.data(), pmix.size() * 16))
      return 2;
    have_check = read_file(d + "/check.bin", gpu_check.data(), gpu_check.size() * 4);
    if (argc > 3) chunk_log2 = atoi(argv[3]);
  } else {
    fprintf(stderr, "unknown mode %s\n", mode.c_str());
    return 2;
  }
  static_assert(sizeof(Fp) == 4 && sizeof(FpExt) == 16, "layout");
  Fp* args[4] = {accum.data(), data.data(), out.data(), mix.data()}; // accum, data, global, mix (as the CPU prover)
  size_t chunk = std::min(domain, size_t(1) << chunk_log2);
  std::vector<Fp> carry(std::max<size_t>(split_ncols, 1) * chunk);
  split_carry = carry.data();
  split_stride = chunk;
  std::vector<FpExt> stock(domain), split(domain);
#pragma omp parallel for schedule(static)
  for (size_t r = 0; r < domain; r++)
    stock[r] = poly_fp(r, domain, pmix.data(), args);
  for (size_t base = 0; base < domain; base += chunk) {
    for (size_t p = 0; p < split_npieces; p++) {
#pragma omp parallel for schedule(static)
      for (size_t r = base; r < base + chunk; r++) {
        FpExt v = split_pieces[p](r, domain, pmix.data(), args);
        if (p == split_npieces - 1) split[r] = v;
      }
    }
  }
  size_t bad = 0, zero = 0, first_bad = SIZE_MAX;
  for (size_t r = 0; r < domain; r++) {
    if (!same(stock[r], split[r])) { if (!bad) first_bad = r; bad++; }
    if (stock[r] == FpExt(0)) zero++;
  }
  printf("rows %zu pieces %zu carry_cols %zu chunk %zu: split==stock on %zu rows, MISMATCH on %zu rows (stock tot==0 on %zu rows)\n",
         domain, split_npieces, split_ncols, chunk, domain - bad, bad, zero);
  if (bad) printf("first mismatch at row %zu\n", first_bad);
  size_t cbad = 0;
  if (have_check) {
    // check[] as the stock CUDA kernel computes it, from the CPU's stock poly_fp: confirms the dump is the GPU's input
    Fp rou;
    memcpy(&rou, &rou_raw, 4);  // raw Montgomery word, as the GPU holds it
    for (size_t r = 0; r < domain; r++) {
      Fp x = pow(rou, r);
      Fp y = pow(Fp(3) * x, size_t(1) << po2);
      FpExt ret = stock[r] * inv(y - Fp(1));
      for (int k = 0; k < 4; k++)
        if (ret.elems[k].asRaw() != gpu_check[domain * k + r].asRaw()) { cbad++; break; }
    }
    printf("CPU stock check[] vs GPU stock check[]: %zu of %zu rows differ\n", cbad, domain);
  }
  bool ok = bad == 0 && cbad == 0;
  printf("%s\n", ok ? "SPLIT_CPU_OK" : "SPLIT_CPU_FAIL");
  return ok ? 0 : 1;
}
