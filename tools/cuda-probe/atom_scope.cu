// hazync#727 trace 4: does THIS card + driver refuse to load a module that contains SYSTEM-scope atomics?
//
// Built twice by cuda-probe.yml, as two separate exes so each is its own module:
//     atom_sys.exe   -DHZ_SCOPE=::cuda::thread_scope_system   (what `::cuda::atomic<T>` defaults to)
//     atom_dev.exe   -DHZ_SCOPE=::cuda::thread_scope_device   (the trace 4 fix in rv32im's tables.h)
// Each also has a plain kernel with no atomics. On a card that loads the module, both kernels run and
// the counter reaches 256. On a refused module EVERY launch in it fails, the plain one included --
// that is the signature trace 3 saw in the rv32im module (hz_nop: 801).
//
// Exit code 0 whatever the card says: a refusal is an answer.
#include <cstdio>
#include <cuda/atomic>

#ifndef HZ_SCOPE
#error "build with -DHZ_SCOPE=::cuda::thread_scope_system or ::cuda::thread_scope_device"
#endif
#define HZ_STR2(x) #x
#define HZ_STR(x) HZ_STR2(x)

__global__ void plain(unsigned* out) {
    if (threadIdx.x == 0 && blockIdx.x == 0) *out = 7;
}

__global__ void count(::cuda::atomic<unsigned, HZ_SCOPE>* c) {
    (*c)++;  // exactly the operation in rv32im's LookupTables::lookupDelta
}

static void say(const char* what, cudaError_t e) {
    std::printf("  %-44s %3d %s\n", what, (int)e, e == cudaSuccess ? "cudaSuccess" : cudaGetErrorName(e));
}

int main() {
    std::printf("atom_scope: scope = %s\n", HZ_STR(HZ_SCOPE));
    int hna = -1;
    cudaDeviceGetAttribute(&hna, cudaDevAttrHostNativeAtomicSupported, 0);
    std::printf("  hostNativeAtomicSupported = %d\n", hna);
    unsigned* d = nullptr;
    say("cudaMalloc", cudaMalloc(&d, sizeof(unsigned)));
    if (!d) return 0;
    cudaMemset(d, 0, sizeof(unsigned));
    cudaGetLastError();

    plain<<<1, 32>>>(d);
    cudaError_t le = cudaGetLastError();
    cudaError_t se = cudaDeviceSynchronize();
    unsigned h = 0;
    cudaMemcpy(&h, d, sizeof h, cudaMemcpyDeviceToHost);
    say("plain kernel (no atomics) launch", le);
    say("  sync", se);
    std::printf("  value %u (want 7)\n", h);

    cudaMemset(d, 0, sizeof(unsigned));
    cudaGetLastError();
    count<<<1, 256>>>(reinterpret_cast<::cuda::atomic<unsigned, HZ_SCOPE>*>(d));
    le = cudaGetLastError();
    se = cudaDeviceSynchronize();
    h = 0;
    cudaMemcpy(&h, d, sizeof h, cudaMemcpyDeviceToHost);
    say("atomic ++ kernel launch", le);
    say("  sync", se);
    std::printf("  value %u (want 256)\n", h);
    std::printf("  VERDICT: %s\n", (le == cudaSuccess && h == 256) ? "this module LOADS and runs"
                                                                   : "this module is REFUSED");
    return 0;
}
