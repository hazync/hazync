/* engine/ci/msvc_builtins.h: force-included into every nvcc compile on Windows (NVCC_APPEND_FLAGS --pre-include, set by
 * .github/workflows/engine-windows.yml). OpenVM's CUDA sources use GCC's __builtin_clz / __builtin_ctz (stark-backend
 * stacked_reduction.cu, sumcheck.cu, logup_zerocheck/gkr.cu; openvm vm merkle_tree.cu), which MSVC does not have.
 * Under MSVC only: device code gets CUDA's own __clz / __ffs, host code MSVC's _BitScanReverse / _BitScanForward.
 * Same results for a nonzero argument, which is the only way those call sites use them (both are undefined for 0 in
 * GCC too). Elsewhere (Linux, GCC) this header is not included and nothing changes. */
#pragma once
#if defined(_MSC_VER)
#include <intrin.h>
/* glibc's <sys/types.h> defines `uint` (unsigned int); OpenVM's CUDA sources use it (histogram.cuh, matrix.cu,
 * poseidon2.cuh: 32 uses). MSVC has no such name. (No ulong / ushort uses, so those are left out: glibc's ulong is
 * 64-bit where MSVC's unsigned long is 32, and defining it wrongly would be worse than an error.) */
typedef unsigned int uint;
static inline int hz_clz_host(unsigned int x) { unsigned long i; return _BitScanReverse(&i, x) ? 31 - (int)i : 32; }
static inline int hz_ctz_host(unsigned int x) { unsigned long i; return _BitScanForward(&i, x) ? (int)i : 32; }
#  if defined(__CUDA_ARCH__)
#    define __builtin_clz(x) __clz((int)(x))
#    define __builtin_ctz(x) (__ffs((int)(x)) - 1)
#  else
#    define __builtin_clz(x) hz_clz_host((unsigned int)(x))
#    define __builtin_ctz(x) hz_ctz_host((unsigned int)(x))
#  endif
#endif
