// Copyright 2024 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#include "eval_check.cuh"

#include "cuda_util.h"
#include "supra/fp.h"

#include <exception>

// HAZYNC #727 TRACE (HAZYNC_CUDA_TRACE=1). The 11-14 trace on a GTX 1050 Ti found error 801 ("operation not supported")
// pending TWICE: before the first NTT, and again before accum's inclusive_scan after it had been cleared. A refused
// kernel LAUNCH leaves its error for cudaGetLastError() while the following cudaStreamSynchronize() reports success,
// so check (and CLEAR) after every launch here and name it. Clearing also shows whether the stale error was the only
// thing Thrust tripped on. Trace-only: without the variable nothing changes.
#include <cstdio>
#include <cstdlib>
static inline bool hz_trace_on() { return std::getenv("HAZYNC_CUDA_TRACE") != nullptr; }
static inline void hz_trace_err(const char* where) {
  if (!hz_trace_on()) return;
  cudaError_t e = cudaGetLastError();
  std::fprintf(stderr, "HAZYNC TRACE %s: %d (%s)\n", where, (int)e, cudaGetErrorString(e));
  std::fflush(stderr);
}
static inline void hz_trace_caps() {
  static bool done = false;
  if (!hz_trace_on() || done) return;
  done = true;
  int dev = -1; cudaGetDevice(&dev);
  int v[6] = {-1, -1, -1, -1, -1, -1};
  cudaDeviceGetAttribute(&v[0], cudaDevAttrManagedMemory, dev);
  cudaDeviceGetAttribute(&v[1], cudaDevAttrConcurrentManagedAccess, dev);
  cudaDeviceGetAttribute(&v[2], cudaDevAttrMemoryPoolsSupported, dev);
  cudaDeviceGetAttribute(&v[3], cudaDevAttrCooperativeLaunch, dev);
  cudaDeviceGetAttribute(&v[4], cudaDevAttrComputePreemptionSupported, dev);
  cudaDeviceGetAttribute(&v[5], cudaDevAttrPageableMemoryAccess, dev);
  std::fprintf(stderr, "HAZYNC TRACE caps dev=%d managed=%d concurrentManaged=%d memPools=%d coop=%d preempt=%d pageable=%d\n",
               dev, v[0], v[1], v[2], v[3], v[4], v[5]);
  std::fflush(stderr);
}

namespace risc0::circuit::rv32im_v2::cuda {

__constant__ FpExt poly_mix[kNumPolyMixPows];

__global__ void eval_check(Fp* check,
                           const Fp* ctrl,
                           const Fp* data,
                           const Fp* accum,
                           const Fp* mix,
                           const Fp* out,
                           const Fp rou,
                           uint32_t po2,
                           uint32_t domain) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  if (cycle < domain) {
    FpExt tot = poly_fp(cycle, domain, ctrl, out, data, mix, accum);
    Fp x = pow(rou, cycle);
    Fp y = pow(Fp(3) * x, 1 << po2);
    FpExt ret = tot * inv(y - Fp(1));
    check[domain * 0 + cycle] = ret[0];
    check[domain * 1 + cycle] = ret[1];
    check[domain * 2 + cycle] = ret[2];
    check[domain * 3 + cycle] = ret[3];
  }
}

} // namespace risc0::circuit::rv32im_v2::cuda

using namespace risc0::circuit::rv32im_v2::cuda;

extern "C" {

const char* risc0_circuit_rv32im_cuda_eval_check(Fp* check,
                                                 const Fp* ctrl,
                                                 const Fp* data,
                                                 const Fp* accum,
                                                 const Fp* mix,
                                                 const Fp* out,
                                                 const Fp& rou,
                                                 uint32_t po2,
                                                 uint32_t domain,
                                                 const FpExt* poly_mix_pows) {
  hz_trace_err("enter eval_check (pending from earlier code)");
  try {
    CUDA_OK(cudaDeviceSynchronize());

    CudaStream stream;
    hz_trace_err("eval_check: CudaStream");
    auto cfg = getSimpleConfig(domain);
    cudaMemcpyToSymbol(poly_mix, poly_mix_pows, sizeof(poly_mix));
    hz_trace_err("eval_check: cudaMemcpyToSymbol (unchecked upstream)");
    eval_check<<<cfg.grid, cfg.block, 0, stream>>>(
        check, ctrl, data, accum, mix, out, rou, po2, domain);
    hz_trace_err("launch eval_check");
    CUDA_OK(cudaStreamSynchronize(stream));
  } catch (const std::exception& err) {
    return strdup(err.what());
  } catch (...) {
    return strdup("Generic exception");
  }
  return nullptr;
}

} // extern "C"
