// Copyright 2025 RISC Zero, Inc.
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

#include "context.h"
#include "cuda_util.h"
#include "fp.h"
#include "fpext.h"
#include "kernels.h"

#include "vendor/nvtx3/nvtx3.hpp"

#include <cstdlib>
#include <cstring>
#include <cuda/std/array>
#include <cuda_runtime.h>
#include <exception>
#include <thrust/execution_policy.h>
#include <thrust/host_vector.h>
#include <thrust/sort.h>
#include <vector>

constexpr size_t kStepModeParallel = 0;
constexpr size_t kStepModeSeqForward = 1;
constexpr size_t kStepModeSeqReverse = 2;

namespace sppark {
void calcPrefixProducts(void* d_inout, uint32_t count);
} // namespace sppark

__device__ void nextStepExec(ExecContext* ctx, uint32_t cycle, uint32_t count) {
  if (cycle == 0 || ctx->isParSafeExec(cycle)) {
    Fp* args[3]{ctx->buffers.ctrl, ctx->buffers.global, ctx->buffers.data};
    // printf("step_exec(%u)\n", cycle);
    step_exec(ctx, ctx->totalCycles, cycle++, args[0], args[1], args[2], nullptr, nullptr);
    while (cycle < count && !ctx->isParSafeExec(cycle)) {
      // printf("  step_exec(%u)\n", cycle);
      step_exec(ctx, ctx->totalCycles, cycle++, args[0], args[1], args[2], nullptr, nullptr);
    }
  }
}

__global__ void parStepExec(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle < count) {
    nextStepExec(ctx, cycle, count);
  }
}

__global__ void fwdStepExec(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle == 0) {
    Fp* args[3]{ctx->buffers.ctrl, ctx->buffers.global, ctx->buffers.data};
    for (uint32_t cycle = 0; cycle < count; cycle++) {
      // printf("step_exec(%u)\n", cycle);
      step_exec(ctx, ctx->totalCycles, cycle, args[0], args[1], args[2], nullptr, nullptr);
    }
  }
}

__global__ void revStepExec(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle == count - 1) {
    Fp* args[3]{ctx->buffers.ctrl, ctx->buffers.global, ctx->buffers.data};
    for (uint32_t i = 0; i < count; i++) {
      uint32_t cycle = count - i - 1;
      nextStepExec(ctx, cycle, count);
    }
  }
}

__global__ void injectWomBacks(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle < 1 || cycle >= count) {
    return;
  }

  Fp* data = ctx->buffers.data;
  uint32_t totalCycles = ctx->totalCycles;
  uint32_t idx = ctx->womIndex[cycle];
  if (idx) {
    const WomArgumentRow& prev = ctx->womRows[idx - 1];
    data[0 * totalCycles + cycle - 1] = prev.addr;
    data[1 * totalCycles + cycle - 1] = prev.value.elems[0];
    data[2 * totalCycles + cycle - 1] = prev.value.elems[1];
    data[3 * totalCycles + cycle - 1] = prev.value.elems[2];
    data[4 * totalCycles + cycle - 1] = prev.value.elems[3];
  } else {
    data[0 * totalCycles + cycle - 1] = 0;
    data[1 * totalCycles + cycle - 1] = 0;
    data[2 * totalCycles + cycle - 1] = 0;
    data[3 * totalCycles + cycle - 1] = 0;
    data[4 * totalCycles + cycle - 1] = 0;
  }
}

__global__ void parStepVerifyWom(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle < count) {
    Fp* args[3]{ctx->buffers.ctrl, ctx->buffers.global, ctx->buffers.data};
    step_verify_mem(ctx, ctx->totalCycles, cycle, args[0], args[1], args[2], nullptr, nullptr);
  }
}

__global__ void fwdStepVerifyWom(ExecContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  uint32_t count = ctx->trace->numCycles;
  if (cycle == 0) {
    Fp* args[3]{ctx->buffers.ctrl, ctx->buffers.global, ctx->buffers.data};
    for (uint32_t cycle = 0; cycle < count; cycle++) {
      // printf("step_verify_mem: %u\n", cycle);
      step_verify_mem(ctx, ctx->totalCycles, cycle, args[0], args[1], args[2], nullptr, nullptr);
    }
  }
}

__global__ void parStepComputeAccum(AccumContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  if (cycle < ctx->workCycles) {
    step_compute_accum(ctx,
                       ctx->totalCycles,
                       cycle,
                       ctx->buffers.ctrl,
                       ctx->buffers.global,
                       ctx->buffers.data,
                       ctx->buffers.mix,
                       ctx->buffers.accum);
  }
}

__global__ void parStepVerifyAccum(AccumContext* ctx) {
  uint32_t cycle = blockDim.x * blockIdx.x + threadIdx.x;
  if (cycle < ctx->workCycles) {
    step_verify_accum(ctx,
                      ctx->totalCycles,
                      cycle,
                      ctx->buffers.ctrl,
                      ctx->buffers.global,
                      ctx->buffers.data,
                      ctx->buffers.mix,
                      ctx->buffers.accum);
  }
}

struct HostExecContext {
  // HAZYNC #727 CANDIDATE FIX, A/B under HAZYNC_727_DEVICE_CTX=1: ExecContext and its PreflightTrace are filled
  // on the host and copied into ORDINARY device memory instead of living in cudaMallocManaged memory. The kernels
  // read the same bytes. Without the variable this is the original managed path. (ExecContext declares a
  // constructor, so its host mirror is raw aligned storage, exactly as cudaMallocManaged's memory was.)
  ExecContext* ctx;   // what the kernels are given
  alignas(ExecContext) unsigned char h_raw[sizeof(ExecContext)];
  PreflightTrace h_trace;
  ExecContext* host;     // host-writable view: managed memory itself, or the mirror
  PreflightTrace* htrace;
  bool dev;
  CudaStream stream;
  LaunchConfig cfg;

  HostExecContext(ExecBuffers* buffers, PreflightTrace* trace, size_t totalCycles)
      : cfg(getSimpleConfig(trace->numCycles)) {
    dev = std::getenv("HAZYNC_727_DEVICE_CTX") != nullptr;
    if (dev) {
      std::memset(h_raw, 0, sizeof(h_raw));
      std::memset(&h_trace, 0, sizeof(h_trace));
      CUDA_OK(cudaMalloc(&ctx, sizeof(ExecContext)));
      host = reinterpret_cast<ExecContext*>(h_raw);
    } else {
      CUDA_OK(cudaMallocManaged(&ctx, sizeof(ExecContext)));
      host = ctx;
    }
    host->buffers.ctrl = buffers->ctrl;
    host->buffers.data = buffers->data;
    host->buffers.global = buffers->global;
    host->totalCycles = totalCycles;

    if (dev) {
      CUDA_OK(cudaMalloc(&host->trace, sizeof(PreflightTrace)));
      htrace = &h_trace;
    } else {
      CUDA_OK(cudaMallocManaged(&ctx->trace, sizeof(PreflightTrace)));
      htrace = ctx->trace;
    }
    htrace->numWoms = trace->numWoms;
    htrace->numCycles = trace->numCycles;
    htrace->numIops = trace->numIops;

    CUDA_OK(cudaMalloc(&htrace->wom, trace->numWoms * sizeof(FpExt)));
    CUDA_OK(cudaMemcpy(
        htrace->wom, trace->wom, trace->numWoms * sizeof(FpExt), cudaMemcpyHostToDevice));

    CUDA_OK(cudaMalloc(&htrace->cycles, trace->numCycles * sizeof(PreflightCycle)));
    CUDA_OK(cudaMemcpy(htrace->cycles,
                       trace->cycles,
                       trace->numCycles * sizeof(PreflightCycle),
                       cudaMemcpyHostToDevice));

    CUDA_OK(cudaMalloc(&htrace->iops, trace->numIops * sizeof(FpExt)));
    CUDA_OK(cudaMemcpy(
        htrace->iops, trace->iops, trace->numIops * sizeof(FpExt), cudaMemcpyHostToDevice));

    CUDA_OK(
        cudaMalloc(&host->womRows, trace->numCycles * kMaxWomRowsPerCycle * sizeof(WomArgumentRow)));
    CUDA_OK(cudaMemset(host->womRows,
                       kInvalidPattern,
                       trace->numCycles * kMaxWomRowsPerCycle * sizeof(WomArgumentRow)));

    CUDA_OK(cudaMalloc(&host->womIndex, trace->numCycles * sizeof(uint32_t)));
    CUDA_OK(cudaMemset(host->womIndex, 0, trace->numCycles * sizeof(uint32_t)));
    if (dev) {
      CUDA_OK(cudaMemcpy(host->trace, htrace, sizeof(PreflightTrace), cudaMemcpyHostToDevice));
      CUDA_OK(cudaMemcpy(ctx, host, sizeof(ExecContext), cudaMemcpyHostToDevice));
    }
  }

  ~HostExecContext() {
    cudaFree(host->womIndex);
    cudaFree(host->womRows);
    cudaFree(htrace->iops);
    cudaFree(htrace->cycles);
    cudaFree(htrace->wom);
    cudaFree(host->trace);
    cudaFree(ctx);
  }

  void doStepExec(uint32_t mode) {
    nvtx3::scoped_range range("stepExec");
    switch (mode) {
    case kStepModeParallel: {
      parStepExec<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
    } break;
    case kStepModeSeqForward: {
      fwdStepExec<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
    } break;
    case kStepModeSeqReverse: {
      revStepExec<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
    } break;
    }
    CUDA_OK(cudaStreamSynchronize(stream));
  }

  void verifyWom(uint32_t mode) {
    nvtx3::scoped_range range("verifyWom");
    uint32_t numCycles = htrace->numCycles;

    {
      nvtx3::scoped_range range("sortWom");
      thrust::sort(thrust::device, host->womRows, host->womRows + numCycles * kMaxWomRowsPerCycle);
    }

    {
      nvtx3::scoped_range range("scan");
      thrust::exclusive_scan(
          thrust::device, host->womIndex, host->womIndex + numCycles, host->womIndex);
    }

    {
      nvtx3::scoped_range range("injectWomBacks");
      injectWomBacks<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
      CUDA_OK(cudaStreamSynchronize(stream));
    }

    {
      nvtx3::scoped_range range("stepVerifyWom");
      parStepVerifyWom<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
      CUDA_OK(cudaStreamSynchronize(stream));
    }
  }
};

struct HostAccumContext {
  // HAZYNC #727 CANDIDATE FIX: see HostExecContext.
  AccumContext* ctx;   // what the kernels are given
  AccumContext h_ctx;  // host mirror (device mode)
  AccumContext* host;  // host-writable view: managed memory itself, or the mirror
  bool dev;
  CudaStream stream;
  LaunchConfig cfg;

  HostAccumContext(AccumBuffers* buffers, size_t workCycles, size_t totalCycles)
      : cfg(getSimpleConfig(workCycles)) {
    dev = std::getenv("HAZYNC_727_DEVICE_CTX") != nullptr;
    if (dev) {
      std::memset(&h_ctx, 0, sizeof(h_ctx));
      CUDA_OK(cudaMalloc(&ctx, sizeof(AccumContext)));
      host = &h_ctx;
    } else {
      CUDA_OK(cudaMallocManaged(&ctx, sizeof(AccumContext)));
      host = ctx;
    }
    host->buffers.ctrl = buffers->ctrl;
    host->buffers.global = buffers->global;
    host->buffers.data = buffers->data;
    host->buffers.mix = buffers->mix;
    host->buffers.accum = buffers->accum;
    host->totalCycles = totalCycles;
    host->workCycles = workCycles;

    std::vector<FpExt> accumInit(workCycles, FpExt(1));
    CUDA_OK(cudaMalloc(&host->accum, workCycles * sizeof(FpExt)));
    CUDA_OK(cudaMemcpy(
        host->accum, accumInit.data(), workCycles * sizeof(FpExt), cudaMemcpyHostToDevice));
    if (dev) {
      CUDA_OK(cudaMemcpy(ctx, host, sizeof(AccumContext), cudaMemcpyHostToDevice));
    }
  }

  ~HostAccumContext() {
    cudaFree(host->accum);
    cudaFree(ctx);
  }

  void computeAccum() {
    nvtx3::scoped_range range("computeAccum");
    parStepComputeAccum<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
    CUDA_OK(cudaStreamSynchronize(stream));
  }

  void calcPrefixProducts() {
    nvtx3::scoped_range range("calcPrefixProducts");
    sppark::calcPrefixProducts(host->accum, host->workCycles);
    CUDA_OK(cudaStreamSynchronize(stream));
  }

  void verifyAccum() {
    nvtx3::scoped_range range("verifyAccum");
    CUDA_OK(cudaDeviceSynchronize());
    parStepVerifyAccum<<<cfg.grid, cfg.block, 0, stream>>>(ctx);
    CUDA_OK(cudaStreamSynchronize(stream));
  }
};

extern "C" {

const char* risc0_circuit_recursion_cuda_witgen(uint32_t mode,
                                                ExecBuffers* buffers,
                                                PreflightTrace* trace,
                                                uint32_t totalCycles) {
  try {
    CUDA_OK(cudaDeviceSynchronize());
    HostExecContext ctx(buffers, trace, totalCycles);
    ctx.doStepExec(mode);
    ctx.verifyWom(mode);
  } catch (const std::exception& err) {
    return strdup(err.what());
  }
  return nullptr;
}

const char* risc0_circuit_recursion_cuda_accum(AccumBuffers* buffers,
                                               uint32_t workCycles,
                                               uint32_t totalCycles) {
  try {
    CUDA_OK(cudaDeviceSynchronize());
    HostAccumContext ctx(buffers, workCycles, totalCycles);
    ctx.computeAccum();
    ctx.calcPrefixProducts();
    ctx.verifyAccum();
  } catch (const std::exception& err) {
    return strdup(err.what());
  }
  return nullptr;
}

} // extern "C"
