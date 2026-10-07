// Does THIS card, on THIS platform, actually support what sppark's NTT needs? (hazync#631)
//
// ⛔⛔ WHY THIS EXISTS. #631 has had FOUR "root causes" in three days -- OOM, the device floor,
// memory pools, and missing sm_61 kernels -- and three of them were refuted by something already
// written in the repo. Every one of those rounds cost a 2h16m risc0+sppark build and most of an
// evening. This file compiles in seconds, downloads in a blink, and answers the question directly
// instead of inferring it from a prover that fails for any of a dozen reasons.
//
// It prints ERROR NAMES, not just messages. That distinction is the whole point: a missing kernel
// image is cudaErrorNoKernelImageForDevice, while a refused cooperative launch is
// cudaErrorNotSupported -- whose message is the bare words "operation not supported", exactly what
// the real prover reports. Matching the message alone is how a wrong theory survived a whole round.
//
//     probe.exe            # report every device, then run each launch test
//
// Exit code is 0 whatever the GPU says: a card that cannot do something is an ANSWER, not a
// failure of this program. Only a probe that could not run at all exits non-zero.

#include <cstdio>
#include <cooperative_groups.h>

// Deliberately trivial. If a launch fails it must be because of HOW it was launched or WHAT the
// card is, never because the kernel did something exotic.
__global__ void touch(int* out)
{
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *out = 1;
}

// The shape sppark's NTT actually uses: a grid-wide sync, which REQUIRES a cooperative launch.
// (vendor/sppark/sppark/ntt/kernels.cu:209 does exactly this.)
__global__ void touch_grid_sync(int* out)
{
    cooperative_groups::this_grid().sync();
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *out = 2;
}

static void say(const char* what, cudaError_t e)
{
    // ⭐ NAME FIRST. "operation not supported" is ambiguous prose; cudaErrorNotSupported is not.
    std::printf("  %-34s %-34s %s\n", what,
                e == cudaSuccess ? "OK" : cudaGetErrorName(e),
                e == cudaSuccess ? "" : cudaGetErrorString(e));
}

int main()
{
    int n = 0;
    cudaError_t e = cudaGetDeviceCount(&n);
    if (e != cudaSuccess) {
        std::printf("cudaGetDeviceCount failed: %s (%s)\n", cudaGetErrorName(e), cudaGetErrorString(e));
        return 1;
    }
    std::printf("CUDA devices: %d\n", n);

    int rt = 0, drv = 0;
    cudaRuntimeGetVersion(&rt);
    cudaDriverGetVersion(&drv);
    std::printf("runtime %d.%d, driver %d.%d\n\n", rt / 1000, (rt % 1000) / 10, drv / 1000, (drv % 1000) / 10);

    for (int id = 0; id < n; id++) {
        cudaDeviceProp p;
        if (cudaGetDeviceProperties(&p, id) != cudaSuccess) {
            std::printf("[%d] properties unreadable\n", id);
            continue;
        }
        std::printf("[%d] %s\n", id, p.name);
        std::printf("  compute capability               %d.%d\n", p.major, p.minor);
        std::printf("  total VRAM                       %.2f GB\n", (double)p.totalGlobalMem / (1024.0 * 1024.0 * 1024.0));
        std::printf("  SMs                              %d\n", p.multiProcessorCount);
        // ⚠ THIS PROPERTY IS THE SUSPECT. sppark's device filter accepts a card when this is
        // non-zero -- but the property is a statement about the DEVICE, and the launch can still be
        // refused by the PLATFORM (Windows WDDM). Printing both the claim and the outcome is the
        // only way to catch a card that advertises what the driver will not do.
        std::printf("  cooperativeLaunch (CLAIMED)      %d\n", p.cooperativeLaunch);
        std::printf("  computeMode                      %d\n", p.computeMode);

        if (cudaSetDevice(id) != cudaSuccess) {
            std::printf("  cudaSetDevice failed — skipping launch tests\n\n");
            continue;
        }

        int* d = nullptr;
        say("cudaMalloc", cudaMalloc(&d, sizeof(int)));

        // 1. ORDINARY LAUNCH. If this fails with cudaErrorNoKernelImageForDevice then the binary
        //    genuinely has no code for this card, and that IS the missing-sm_61 theory.
        touch<<<1, 32>>>(d);
        say("plain launch <<<1,32>>>", cudaGetLastError());
        say("  then synchronize", cudaDeviceSynchronize());

        // 2. COOPERATIVE LAUNCH, trivial kernel. Separates "cooperative launch is refused" from
        //    "the grid sync itself is the problem".
        void* args[] = { &d };
        say("cudaLaunchCooperativeKernel",
            cudaLaunchCooperativeKernel((const void*)touch, dim3(1), dim3(32), args, 0, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 3. THE REAL SHAPE: a grid-wide sync, sized so it cannot fail merely for being too large.
        //    ⚠ A cooperative launch requires every block to be CO-RESIDENT, so the grid is capped
        //    at occupancy x SMs. Oversizing it yields cudaErrorCooperativeLaunchTooLarge, which
        //    would be a self-inflicted answer rather than a fact about the card.
        int per_sm = 0;
        say("occupancy query",
            cudaOccupancyMaxActiveBlocksPerMultiprocessor(&per_sm, touch_grid_sync, 32, 0));
        int blocks = per_sm * p.multiProcessorCount;
        if (blocks < 1) blocks = 1;
        std::printf("  co-resident blocks usable        %d (%d/SM x %d SMs)\n", blocks, per_sm, p.multiProcessorCount);
        say("coop launch + this_grid().sync()",
            cudaLaunchCooperativeKernel((const void*)touch_grid_sync, dim3(blocks), dim3(32), args, 0, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 4. ⭐⭐ STREAM-ORDERED ALLOCATION. sppark calls cudaMallocAsync UNCONDITIONALLY
        //    (gpu_t.cuh:73 and :339) and never asks whether this device has memory pools. On a
        //    device where it does not, cudaMallocAsync returns cudaErrorNotSupported -- whose
        //    message is the bare words "operation not supported", which is what the prover reports.
        //    ⚠ And cudaGetLastError() is STICKY: a failure here would surface at the NEXT check,
        //    which in the NTT path is ntt.cuh:97 -- a line that has nothing to do with allocation.
        int pools = -1;
        say("query cudaDevAttrMemoryPoolsSupported",
            cudaDeviceGetAttribute(&pools, cudaDevAttrMemoryPoolsSupported, id));
        std::printf("  memoryPoolsSupported             %d\n", pools);
        int* da = nullptr;
        say("cudaMallocAsync (stream 0)", cudaMallocAsync(&da, sizeof(int), 0));
        say("cudaFreeAsync", cudaFreeAsync(da, 0));
        say("  then synchronize", cudaDeviceSynchronize());

        // 5. THE LAUNCH SHAPE THE NTT ACTUALLY USES: ntt.cuh:94 is
        //    `LDE_distribute_powers<<<stream.sm_count(), 1024, 0, stream>>>`. 1024 threads is the
        //    Pascal maximum, and a block that large can be refused for resources where 32 is fine.
        //    My first probe used 32 and therefore could not have caught this.
        touch<<<p.multiProcessorCount, 1024>>>(d);
        say("plain launch <<<SMs,1024>>>", cudaGetLastError());
        say("  then synchronize", cudaDeviceSynchronize());

        cudaFree(d);
        std::printf("\n");
    }

    std::printf("done — a refusal above is an ANSWER, not a crash\n");
    return 0;
}
