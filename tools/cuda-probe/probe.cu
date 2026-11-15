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
#include <cstdlib>
#include <cstring>
#include <exception>
#include <vector>
#include <cooperative_groups.h>
#include <cub/device/device_scan.cuh>
#include <thrust/execution_policy.h>
#include <thrust/scan.h>

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

// ============================================================================================
// hazync#727 TRACE 3 — what makes a launch fail with 801 on this card?
//
// Trace 2 on a GTX 1050 Ti (WDDM, driver 566.14): every launch made while risc0's tiny
// cudaMallocManaged context struct was ALIVE failed with 801 cudaErrorNotSupported
// (par_stepExec x2, stepAccum, Thrust's inclusive_scan), and every launch made while none was
// alive succeeded (212 sppark LDE/NTT launches, risc0-sys eltwise/hash kernels). The prover's
// HAL buffers are plain device memory (cust DeviceBuffer = cuMemAlloc); the only managed memory
// on the whole GPU path is those context structs.
//
// So this section asks, one variable at a time:
//   C*  CONTROLS — known-good patterns with NO managed memory alive. These are exactly what
//       already WORKED on this card in trace 2 (plain launches on blocking / non-blocking /
//       default streams, cudaLaunchKernelEx, cudaMalloc). If a control fails, the PROBE is the
//       suspect, not the card, and the verdict says so.
//   M*  the same launches while a managed allocation merely EXISTS (kernel never touches it);
//       then a kernel that DOES read/write managed memory; then the risc0 shape exactly (a
//       managed struct holding device pointers, written on the host, dereferenced on the device).
//   S*  managed sizes: 1 MiB, 1 GiB, and larger than free VRAM.
//   A*  cudaMemAttachHost + cudaStreamAttachMemAsync (legacy-UM stream attachment).
//   F*  candidate FIX shapes: the struct in device memory (cudaMalloc + cudaMemcpy), and in pinned
//       host memory (cudaMallocHost). F1 is the fix trace 3's host.exe applies under
//       HAZYNC_727_DEVICE_CTX=1.
//   X*  Thrust inclusive_scan and cub::DeviceScan on cudaMalloc / cudaMallocAsync / managed data,
//       with and without a managed allocation alive.
//
// Every kernel writes a SENTINEL that is copied back and compared, so "the launch returned OK but
// nothing ran" cannot pass as OK. After every step a CANARY (plain launch, device memory, no
// managed alive) checks the context is still healthy; a sticky error stops the section, because
// later answers would be about a dead context rather than about the card.
// ============================================================================================

struct Hz727Ctx {
    int* out;
    int value;
};

__global__ void hz_write(int* out, int v)
{
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *out = v;
}

// risc0's shape: the kernel gets a pointer to a small struct and dereferences it for its buffers.
__global__ void hz_write_via_ctx(Hz727Ctx* ctx)
{
    if (threadIdx.x == 0 && blockIdx.x == 0)
        *ctx->out = ctx->value;
}

// touches the first and last int of a (possibly huge) buffer
__global__ void hz_touch_ends(int* buf, size_t n, int v)
{
    if (threadIdx.x == 0 && blockIdx.x == 0) {
        buf[0] = v;
        buf[n - 1] = v;
    }
}

static int g727_fail = 0;      // steps that refused
static int g727_ctrl_fail = 0; // CONTROLS that refused (probe suspect)
static bool g727_dead = false; // context poisoned (sticky)
static bool g727_controls_ok = false;
static int m_exist_fail = -1, m_touch_fail = -1, m_ctx_fail = -1, m_scan_fail = -1, recover_fail = -1;
static int attach_host_fail = -1, attach_single_fail = -1, fix_dev_fail = -1, fix_pinned_fail = -1;
static int scan_pool_fail = -1, scan_managed_fail = -1, coop_plain_fail = -1, coop_managed_fail = -1;

static void r727(const char* id, const char* what, cudaError_t e, const char* note = "")
{
    std::printf("  %-4s %-58s %3d %-30s %s\n", id, what, (int)e,
                e == cudaSuccess ? "cudaSuccess" : cudaGetErrorName(e), note);
    std::fflush(stdout);
}

// Launch hz_write on `s`, sync, copy back, compare. Returns the FIRST failing error, or a
// synthetic cudaErrorUnknown if everything returned OK but the sentinel did not arrive.
static cudaError_t launch_write(int* d, int v, cudaStream_t s, bool ex, cudaError_t* launch_out,
                                cudaError_t* sync_out, bool* value_ok)
{
    cudaGetLastError();
    cudaMemset(d, 0, sizeof(int));
    cudaDeviceSynchronize();
    cudaError_t le;
    if (ex) {
        cudaLaunchConfig_t cfg;
        std::memset(&cfg, 0, sizeof(cfg));
        cfg.gridDim = dim3(1);
        cfg.blockDim = dim3(256);
        cfg.dynamicSmemBytes = 0;
        cfg.stream = s;
        cfg.attrs = nullptr;
        cfg.numAttrs = 0;
        le = cudaLaunchKernelEx(&cfg, hz_write, d, v);
    } else {
        hz_write<<<1, 256, 0, s>>>(d, v);
        le = cudaGetLastError();
    }
    cudaError_t se = cudaStreamSynchronize(s);
    int h = 0;
    cudaMemcpy(&h, d, sizeof(int), cudaMemcpyDeviceToHost);
    cudaGetLastError();
    *launch_out = le;
    *sync_out = se;
    *value_ok = (h == v);
    if (le != cudaSuccess) return le;
    if (se != cudaSuccess) return se;
    return h == v ? cudaSuccess : cudaErrorUnknown;
}

static void step_write(const char* id, const char* what, int* d, int v, cudaStream_t s, bool ex,
                       bool is_control)
{
    cudaError_t le, se;
    bool ok;
    cudaError_t e = launch_write(d, v, s, ex, &le, &se, &ok);
    char note[160];
    std::snprintf(note, sizeof note, "launch=%s sync=%s value=%s%s", cudaGetErrorName(le),
                  cudaGetErrorName(se), ok ? "ok" : "WRONG",
                  (e != cudaSuccess && is_control) ? "  <- CONTROL FAILED: suspect the probe" : "");
    r727(id, what, e, note);
    if (e != cudaSuccess) {
        g727_fail++;
        if (is_control) g727_ctrl_fail++;
    }
}

// The canary: plain launch, device memory, its own fresh blocking stream. Must always pass when
// no managed memory is alive. Returns false (and marks the context dead) on a STICKY error.
static bool canary(int* d, const char* after)
{
    cudaStream_t s = nullptr;
    cudaStreamCreate(&s);
    cudaError_t le, se;
    bool ok;
    cudaError_t e = launch_write(d, 0x5a5a, s, false, &le, &se, &ok);
    cudaStreamDestroy(s);
    if (e != cudaSuccess) {
        // Is it sticky? cudaGetLastError clears non-sticky errors; a sticky one comes back.
        cudaGetLastError();
        cudaError_t again = cudaDeviceSynchronize();
        std::printf("  !!   canary after %s: %d %s (device sync now: %s)\n", after, (int)e,
                    cudaGetErrorName(e), cudaGetErrorName(again));
        if (again != cudaSuccess) {
            g727_dead = true;
            std::printf("  !!   CONTEXT POISONED (sticky error) — the rest of this section is not run.\n");
            return false;
        }
    }
    return true;
}

// Thrust inclusive_scan(thrust::device, ...) in place, exactly the call in risc0's accum.
static void step_thrust_scan(const char* id, const char* what, unsigned* data, int n)
{
    std::vector<unsigned> h(n), want(n), got(n);
    for (int i = 0; i < n; i++) h[i] = (unsigned)(i % 7 + 1);
    unsigned acc = 0;
    for (int i = 0; i < n; i++) { acc += h[i]; want[i] = acc; }
    cudaGetLastError();
    cudaError_t e = cudaMemcpy(data, h.data(), n * sizeof(unsigned), cudaMemcpyHostToDevice);
    const char* exc = "";
    static char buf[200];
    if (e == cudaSuccess) {
        try {
            thrust::inclusive_scan(thrust::device, data, data + n, data);
            e = cudaDeviceSynchronize();
        } catch (const std::exception& x) {
            std::snprintf(buf, sizeof buf, "thrust threw: %.150s", x.what());
            exc = buf;
            e = cudaGetLastError();
            if (e == cudaSuccess) e = cudaErrorUnknown;
        }
    }
    bool ok = false;
    if (e == cudaSuccess) {
        cudaMemcpy(got.data(), data, n * sizeof(unsigned), cudaMemcpyDeviceToHost);
        ok = (got == want);
        if (!ok) e = cudaErrorUnknown;
    }
    cudaGetLastError();
    char note[260];
    std::snprintf(note, sizeof note, "%s%s", ok ? "value=ok " : "", exc);
    r727(id, what, e, note);
    if (e != cudaSuccess) g727_fail++;
}

// cub::DeviceScan::InclusiveSum on a stream, temp storage from cudaMalloc.
static void step_cub_scan(const char* id, const char* what, unsigned* data, int n, cudaStream_t s)
{
    std::vector<unsigned> h(n), want(n), got(n);
    for (int i = 0; i < n; i++) h[i] = (unsigned)(i % 5 + 2);
    unsigned acc = 0;
    for (int i = 0; i < n; i++) { acc += h[i]; want[i] = acc; }
    cudaGetLastError();
    cudaMemcpy(data, h.data(), n * sizeof(unsigned), cudaMemcpyHostToDevice);
    size_t tb = 0;
    cudaError_t e = cub::DeviceScan::InclusiveSum(nullptr, tb, data, data, n, s);
    void* tmp = nullptr;
    if (e == cudaSuccess) e = cudaMalloc(&tmp, tb ? tb : 1);
    if (e == cudaSuccess) e = cub::DeviceScan::InclusiveSum(tmp, tb, data, data, n, s);
    if (e == cudaSuccess) e = cudaGetLastError();
    if (e == cudaSuccess) e = cudaStreamSynchronize(s);
    bool ok = false;
    if (e == cudaSuccess) {
        cudaMemcpy(got.data(), data, n * sizeof(unsigned), cudaMemcpyDeviceToHost);
        ok = (got == want);
        if (!ok) e = cudaErrorUnknown;
    }
    if (tmp) cudaFree(tmp);
    cudaGetLastError();
    r727(id, what, e, ok ? "value=ok" : "");
    if (e != cudaSuccess) g727_fail++;
}

static void probe727_body(int id, const cudaDeviceProp& p)
{
    std::printf("\n  ==== #727 trace 3: managed memory vs launches ====\n");
    int a[12];
    const cudaDeviceAttr attrs[12] = {
        cudaDevAttrManagedMemory, cudaDevAttrConcurrentManagedAccess, cudaDevAttrPageableMemoryAccess,
        cudaDevAttrDirectManagedMemAccessFromHost, cudaDevAttrUnifiedAddressing, cudaDevAttrCanMapHostMemory,
        cudaDevAttrIntegrated, cudaDevAttrTccDriver, cudaDevAttrMemoryPoolsSupported,
        cudaDevAttrHostNativeAtomicSupported, cudaDevAttrCooperativeLaunch, cudaDevAttrComputePreemptionSupported};
    const char* names[12] = {"managedMemory", "concurrentManagedAccess", "pageableMemoryAccess",
                             "directManagedMemAccessFromHost", "unifiedAddressing", "canMapHostMemory",
                             "integrated", "tccDriver", "memoryPoolsSupported", "hostNativeAtomic",
                             "cooperativeLaunch", "computePreemption"};
    for (int i = 0; i < 12; i++) {
        a[i] = -1;
        cudaError_t e = cudaDeviceGetAttribute(&a[i], attrs[i], id);
        std::printf("  attr %-32s %d%s\n", names[i], a[i], e == cudaSuccess ? "" : " (query failed)");
    }
    cudaGetLastError();
    size_t fr = 0, tot = 0;
    r727("I0", "cudaMemGetInfo", cudaMemGetInfo(&fr, &tot));
    std::printf("       free %zu MiB of %zu MiB\n", fr >> 20, tot >> 20);
    const char* lm = std::getenv("CUDA_MODULE_LOADING");
    std::printf("       CUDA_MODULE_LOADING=%s\n", lm ? lm : "(unset)");

    int* d = nullptr;
    cudaError_t e = cudaMalloc(&d, sizeof(int));
    r727("I1", "cudaMalloc 4 B (device buffer used by every launch below)", e);
    if (e != cudaSuccess) { g727_ctrl_fail++; return; }

    cudaStream_t sb = nullptr, snb = nullptr;
    r727("I2", "cudaStreamCreate (blocking; risc0's CudaStream)", cudaStreamCreate(&sb));
    r727("I3", "cudaStreamCreateWithFlags(NonBlocking) (sppark's)", cudaStreamCreateWithFlags(&snb, cudaStreamNonBlocking));

    // ---- CONTROLS: nothing managed alive -------------------------------------------------------
    step_write("C1", "control: <<<>>> on blocking stream", d, 11, sb, false, true);
    step_write("C2", "control: <<<>>> on non-blocking stream", d, 12, snb, false, true);
    step_write("C3", "control: <<<>>> on legacy default stream 0", d, 13, 0, false, true);
    step_write("C4", "control: cudaLaunchKernelEx (risc0-sys launchKernel)", d, 14, sb, true, true);

    const int N = 1 << 16;
    unsigned* sd = nullptr;
    r727("C5a", "cudaMalloc 256 KiB scan data", cudaMalloc(&sd, N * sizeof(unsigned)));
    if (sd) {
        step_thrust_scan("C5", "control: thrust::inclusive_scan(device) on cudaMalloc", sd, N);
        step_cub_scan("C6", "control: cub::DeviceScan::InclusiveSum on cudaMalloc", sd, N, sb);
    }
    if (!canary(d, "controls")) return;
    g727_controls_ok = (g727_ctrl_fail == 0);

    // ---- MANAGED, small, global attach: does its mere EXISTENCE break unrelated launches? ------
    int* m = nullptr;
    e = cudaMallocManaged(&m, 64);
    r727("M0", "cudaMallocManaged 64 B (global attach) — now ALIVE", e);
    if (e == cudaSuccess) {
        int before = g727_fail;
        step_write("M1", "managed alive, kernel ignores it: blocking stream", d, 21, sb, false, false);
        step_write("M2", "managed alive, kernel ignores it: non-blocking stream", d, 22, snb, false, false);
        step_write("M3", "managed alive, kernel ignores it: default stream", d, 23, 0, false, false);
        step_write("M4", "managed alive, kernel ignores it: cudaLaunchKernelEx", d, 24, sb, true, false);
        m_exist_fail = g727_fail - before;

        // kernel WRITES the managed int, host reads it after sync
        before = g727_fail;
        cudaGetLastError();
        hz_write<<<1, 256, 0, sb>>>(m, 25);
        cudaError_t le = cudaGetLastError();
        cudaError_t se = cudaStreamSynchronize(sb);
        cudaError_t de = cudaDeviceSynchronize();  // legacy UM: host may touch managed only when the device is idle
        int got = (le == cudaSuccess && se == cudaSuccess && de == cudaSuccess) ? *m : -1;
        char note[120];
        std::snprintf(note, sizeof note, "launch=%s sync=%s host read=%d (want 25)", cudaGetErrorName(le),
                      cudaGetErrorName(se), got);
        cudaError_t me = le != cudaSuccess ? le : se != cudaSuccess ? se : got == 25 ? cudaSuccess : cudaErrorUnknown;
        r727("M5", "kernel writes the managed buffer", me, note);
        if (me != cudaSuccess) g727_fail++;
        m_touch_fail = g727_fail - before;

        // the risc0 shape: managed struct, host fills it with a DEVICE pointer, kernel derefs it
        before = g727_fail;
        Hz727Ctx* mc = nullptr;
        e = cudaMallocManaged(&mc, sizeof(Hz727Ctx));
        r727("M6a", "cudaMallocManaged ctx struct (risc0 HostExecContext shape)", e);
        if (e == cudaSuccess) {
            cudaDeviceSynchronize();
            mc->out = d;
            mc->value = 26;
            cudaGetLastError();
            cudaMemset(d, 0, sizeof(int));
            cudaDeviceSynchronize();
            hz_write_via_ctx<<<1, 256, 0, sb>>>(mc);
            le = cudaGetLastError();
            se = cudaStreamSynchronize(sb);
            int h = 0;
            cudaMemcpy(&h, d, sizeof(int), cudaMemcpyDeviceToHost);
            std::snprintf(note, sizeof note, "launch=%s sync=%s value=%d (want 26)", cudaGetErrorName(le),
                          cudaGetErrorName(se), h);
            me = le != cudaSuccess ? le : se != cudaSuccess ? se : h == 26 ? cudaSuccess : cudaErrorUnknown;
            r727("M6", "kernel derefs a MANAGED ctx -> device buffer", me, note);
            if (me != cudaSuccess) g727_fail++;
            cudaFree(mc);
            cudaGetLastError();
        }
        m_ctx_fail = g727_fail - before;

        before = g727_fail;
        if (sd) {
            step_thrust_scan("M7", "managed alive: thrust::inclusive_scan on cudaMalloc", sd, N);
            step_cub_scan("M8", "managed alive: cub::DeviceScan on cudaMalloc", sd, N, sb);
        }
        m_scan_fail = g727_fail - before;

        r727("M9", "cudaFree(managed) — nothing managed alive again", cudaFree(m));
        cudaGetLastError();
        before = g727_fail;
        step_write("M10", "after free: <<<>>> blocking stream (recovers?)", d, 27, sb, false, false);
        if (sd) step_thrust_scan("M11", "after free: thrust::inclusive_scan", sd, N);
        recover_fail = g727_fail - before;
    }
    if (!canary(d, "managed small")) return;

    // ---- cudaMemAttachHost + stream attach (legacy-UM escape hatch) ---------------------------
    int* mh = nullptr;
    e = cudaMallocManaged(&mh, 64, cudaMemAttachHost);
    r727("A0", "cudaMallocManaged 64 B, cudaMemAttachHost", e);
    if (e == cudaSuccess) {
        int before = g727_fail;
        step_write("A1", "AttachHost alive, kernel ignores it", d, 31, sb, false, false);
        attach_host_fail = g727_fail - before;
        before = g727_fail;
        cudaError_t ae = cudaStreamAttachMemAsync(sb, mh, 0, cudaMemAttachSingle);
        cudaError_t as = cudaStreamSynchronize(sb);
        char note[100];
        std::snprintf(note, sizeof note, "sync=%s", cudaGetErrorName(as));
        r727("A2", "cudaStreamAttachMemAsync(blocking stream, Single)", ae, note);
        cudaGetLastError();
        hz_write<<<1, 256, 0, sb>>>(mh, 32);
        cudaError_t le = cudaGetLastError();
        cudaError_t se = cudaStreamSynchronize(sb);
        int got = (le == cudaSuccess && se == cudaSuccess) ? *mh : -1;
        std::snprintf(note, sizeof note, "launch=%s sync=%s host read=%d (want 32)", cudaGetErrorName(le),
                      cudaGetErrorName(se), got);
        cudaError_t me = le != cudaSuccess ? le : se != cudaSuccess ? se : got == 32 ? cudaSuccess : cudaErrorUnknown;
        r727("A3", "kernel writes stream-attached managed buffer", me, note);
        if (me != cudaSuccess) g727_fail++;
        attach_single_fail = g727_fail - before;
        cudaFree(mh);
        cudaGetLastError();
    }
    if (!canary(d, "attach")) return;

    // ---- cooperative launch with and without managed alive -------------------------------------
    {
        int v = 0;
        void* args[2] = {&d, &v};
        v = 51;
        cudaGetLastError();
        cudaError_t ce = cudaLaunchCooperativeKernel((const void*)hz_write, dim3(1), dim3(32), args, 0, sb);
        cudaError_t cs = cudaStreamSynchronize(sb);
        char note[60];
        std::snprintf(note, sizeof note, "sync=%s", cudaGetErrorName(cs));
        r727("K1", "cudaLaunchCooperativeKernel, no managed alive", ce, note);
        coop_plain_fail = (ce != cudaSuccess || cs != cudaSuccess) ? 1 : 0;
        int* mk = nullptr;
        if (cudaMallocManaged(&mk, 64) == cudaSuccess) {
            v = 52;
            cudaGetLastError();
            ce = cudaLaunchCooperativeKernel((const void*)hz_write, dim3(1), dim3(32), args, 0, sb);
            cs = cudaStreamSynchronize(sb);
            std::snprintf(note, sizeof note, "sync=%s", cudaGetErrorName(cs));
            r727("K2", "cudaLaunchCooperativeKernel, managed alive", ce, note);
            coop_managed_fail = (ce != cudaSuccess || cs != cudaSuccess) ? 1 : 0;
            cudaFree(mk);
        }
        cudaGetLastError();
    }
    if (!canary(d, "cooperative")) return;

    // ---- FIX SHAPES ----------------------------------------------------------------------------
    {
        int before = g727_fail;
        Hz727Ctx h{d, 61};
        Hz727Ctx* dc = nullptr;
        e = cudaMalloc(&dc, sizeof(Hz727Ctx));
        if (e == cudaSuccess) e = cudaMemcpy(dc, &h, sizeof h, cudaMemcpyHostToDevice);
        r727("F1a", "ctx struct in DEVICE memory (cudaMalloc + cudaMemcpy)", e);
        if (e == cudaSuccess) {
            cudaMemset(d, 0, sizeof(int));
            cudaDeviceSynchronize();
            cudaGetLastError();
            hz_write_via_ctx<<<1, 256, 0, sb>>>(dc);
            cudaError_t le = cudaGetLastError();
            cudaError_t se = cudaStreamSynchronize(sb);
            int got = 0;
            cudaMemcpy(&got, d, sizeof(int), cudaMemcpyDeviceToHost);
            char note[100];
            std::snprintf(note, sizeof note, "launch=%s sync=%s value=%d (want 61)", cudaGetErrorName(le),
                          cudaGetErrorName(se), got);
            cudaError_t me = le != cudaSuccess ? le : se != cudaSuccess ? se : got == 61 ? cudaSuccess : cudaErrorUnknown;
            r727("F1", "FIX: kernel derefs a DEVICE ctx -> device buffer", me, note);
            if (me != cudaSuccess) g727_fail++;
        } else g727_fail++;
        if (dc) cudaFree(dc);
        fix_dev_fail = g727_fail - before;

        before = g727_fail;
        Hz727Ctx* pc = nullptr;
        e = cudaMallocHost(&pc, sizeof(Hz727Ctx));
        r727("F2a", "ctx struct in PINNED host memory (cudaMallocHost)", e);
        if (e == cudaSuccess) {
            pc->out = d;
            pc->value = 62;
            cudaMemset(d, 0, sizeof(int));
            cudaDeviceSynchronize();
            cudaGetLastError();
            hz_write_via_ctx<<<1, 256, 0, sb>>>(pc);
            cudaError_t le = cudaGetLastError();
            cudaError_t se = cudaStreamSynchronize(sb);
            int got = 0;
            cudaMemcpy(&got, d, sizeof(int), cudaMemcpyDeviceToHost);
            char note[100];
            std::snprintf(note, sizeof note, "launch=%s sync=%s value=%d (want 62)", cudaGetErrorName(le),
                          cudaGetErrorName(se), got);
            cudaError_t me = le != cudaSuccess ? le : se != cudaSuccess ? se : got == 62 ? cudaSuccess : cudaErrorUnknown;
            r727("F2", "alt: kernel derefs a PINNED ctx -> device buffer", me, note);
            if (me != cudaSuccess) g727_fail++;
            cudaFreeHost(pc);
        } else g727_fail++;
        cudaGetLastError();
        fix_pinned_fail = g727_fail - before;
    }
    if (!canary(d, "fix shapes")) return;

    // ---- scans on other memory kinds ------------------------------------------------------------
    if (a[8] == 1) {
        unsigned* pd = nullptr;
        e = cudaMallocAsync(&pd, N * sizeof(unsigned), sb);
        cudaError_t s2 = cudaStreamSynchronize(sb);
        r727("X1a", "cudaMallocAsync 256 KiB (memory pool)", e != cudaSuccess ? e : s2);
        if (e == cudaSuccess && s2 == cudaSuccess) {
            int before = g727_fail;
            step_cub_scan("X1", "cub::DeviceScan on cudaMallocAsync memory", pd, N, sb);
            scan_pool_fail = g727_fail - before;
            cudaFreeAsync(pd, sb);
            cudaStreamSynchronize(sb);
        }
        cudaGetLastError();
    }
    {
        unsigned* md = nullptr;
        e = cudaMallocManaged(&md, N * sizeof(unsigned));
        r727("X2a", "cudaMallocManaged 256 KiB scan data", e);
        if (e == cudaSuccess) {
            int before = g727_fail;
            step_thrust_scan("X2", "thrust::inclusive_scan on MANAGED data", md, N);
            scan_managed_fail = g727_fail - before;
            cudaFree(md);
        }
        cudaGetLastError();
    }
    if (!canary(d, "scans")) return;

    // ---- sizes ---------------------------------------------------------------------------------
    // LAST: oversubscribing VRAM is the step most likely to upset the driver.
    cudaMemGetInfo(&fr, &tot);
    const size_t sizes[3] = {(size_t)1 << 20, (size_t)1 << 30, fr + ((size_t)512 << 20)};
    const char* sname[3] = {"1 MiB", "1 GiB", "free VRAM + 512 MiB"};
    for (int k = 0; k < 3; k++) {
        char idb[8], w[96];
        int* big = nullptr;
        std::snprintf(idb, sizeof idb, "S%da", k + 1);
        std::snprintf(w, sizeof w, "cudaMallocManaged %s (%zu MiB)", sname[k], sizes[k] >> 20);
        e = cudaMallocManaged(&big, sizes[k]);
        r727(idb, w, e);
        cudaGetLastError();
        if (e != cudaSuccess) continue;
        size_t f2 = 0, t2 = 0;
        cudaMemGetInfo(&f2, &t2);
        std::printf("       free now %zu MiB\n", f2 >> 20);
        std::snprintf(idb, sizeof idb, "S%db", k + 1);
        std::snprintf(w, sizeof w, "%s managed alive, kernel ignores it", sname[k]);
        step_write(idb, w, d, 40 + k, sb, false, false);
        cudaGetLastError();
        hz_touch_ends<<<1, 32, 0, sb>>>(big, sizes[k] / sizeof(int), 7);
        cudaError_t le = cudaGetLastError();
        cudaError_t se = cudaStreamSynchronize(sb);
        std::snprintf(idb, sizeof idb, "S%dc", k + 1);
        std::snprintf(w, sizeof w, "%s: kernel touches both ends", sname[k]);
        char note[80];
        std::snprintf(note, sizeof note, "sync=%s", cudaGetErrorName(se));
        r727(idb, w, le != cudaSuccess ? le : se, note);
        if (le != cudaSuccess || se != cudaSuccess) g727_fail++;
        cudaFree(big);
        cudaGetLastError();
        if (!canary(d, w)) return;
    }


    if (sd) cudaFree(sd);
    cudaFree(d);
    cudaStreamDestroy(sb);
    cudaStreamDestroy(snb);
    cudaGetLastError();

}

    // ---- VERDICT ---------------------------------------------------------------------------------
static void probe727_verdict(int id, const cudaDeviceProp& p)
{
    std::printf("\n  ---- #727 VERDICT (device %d, %s, cc %d.%d) ----\n", id, p.name, p.major, p.minor);
    if (!g727_controls_ok) {
        std::printf("  PROBE SUSPECT: %d control(s) failed. Those patterns WORKED in trace 2 on this card, so\n"
                    "  the answers below are not trustworthy. Send the file anyway.\n", g727_ctrl_fail);
    } else {
        std::printf("  controls (no managed memory alive): all OK -> the probe works on this card\n");
    }
    auto yn = [](int v) { return v < 0 ? "not run" : v == 0 ? "NO (works)" : "YES (refused)"; };
    std::printf("  managed alloc ALIVE breaks unrelated launches (M1-M4) : %s\n", yn(m_exist_fail));
    std::printf("  kernel writing managed memory refused (M5)            : %s\n", yn(m_touch_fail));
    std::printf("  risc0 shape, managed ctx struct, refused (M6)         : %s\n", yn(m_ctx_fail));
    std::printf("  Thrust/cub scan refused while managed alive (M7-M8)   : %s\n", yn(m_scan_fail));
    std::printf("  launches recover once managed is freed (M10-M11)      : %s\n",
                recover_fail < 0 ? "not run" : recover_fail == 0 ? "YES" : "NO");
    std::printf("  cudaMemAttachHost alloc breaks launches (A1)          : %s\n", yn(attach_host_fail));
    std::printf("  stream-attached managed refused (A2-A3)               : %s\n", yn(attach_single_fail));
    std::printf("  FIX shape: ctx in device memory refused (F1)          : %s\n", yn(fix_dev_fail));
    std::printf("  alt: ctx in pinned host memory refused (F2)           : %s\n", yn(fix_pinned_fail));
    std::printf("  scan on cudaMallocAsync memory refused (X1)           : %s\n", yn(scan_pool_fail));
    std::printf("  scan on managed data refused (X2)                     : %s\n", yn(scan_managed_fail));
    std::printf("  cooperative launch refused: none alive / alive (K1/K2): %s / %s\n", yn(coop_plain_fail),
                yn(coop_managed_fail));
    std::printf("  context poisoned at some point                        : %s\n", g727_dead ? "YES" : "no");
    std::printf("  steps refused in total: %d\n", g727_fail);
}

static void probe727(int id, const cudaDeviceProp& p)
{
    probe727_body(id, p);
    probe727_verdict(id, p);
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

        // #727 FIRST, on a context nothing else has touched yet (see probe727 above).
        probe727(id, p);
        std::printf("\n  ---- legacy #631 launch tests ----\n");

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

        // 6. ⭐⭐ THE CALL THE PROVER ACTUALLY DIES ON: gpu_t.cuh:62 constructs four streams
        //    (stream_t zero + stream_t flipflop[3], all member initialisers, so they run BEFORE the
        //    gpu_t body) with exactly these flags, and reports "operation not supported".
        //    ⚠ stream_t never calls cudaSetDevice, so it builds on whatever device is current.
        cudaStream_t s1 = nullptr;
        say("cudaStreamCreateWithFlags(NonBlocking)",
            cudaStreamCreateWithFlags(&s1, cudaStreamNonBlocking));
        cudaStream_t s4[4] = { nullptr, nullptr, nullptr, nullptr };
        cudaError_t worst = cudaSuccess;
        for (int k = 0; k < 4; k++) {
            cudaError_t ek = cudaStreamCreateWithFlags(&s4[k], cudaStreamNonBlocking);
            if (ek != cudaSuccess && worst == cudaSuccess) worst = ek;
        }
        say("  x4, as gpu_t builds them", worst);
        for (int k = 0; k < 4; k++) if (s4[k]) cudaStreamDestroy(s4[k]);
        if (s1) cudaStreamDestroy(s1);

        cudaFree(d);
        std::printf("\n");
    }

    std::printf("done — a refusal above is an ANSWER, not a crash\n");
    return 0;
}
