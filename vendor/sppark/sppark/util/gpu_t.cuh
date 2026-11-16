// Copyright Supranational LLC
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

#ifndef __SPPARK_UTIL_GPU_T_CUH__
#define __SPPARK_UTIL_GPU_T_CUH__

#if defined(__NVCC__) && !defined(__CUDACC__)
# include <cuda_runtime.h>
#endif

#include <atomic>
#include <cstdio>
#include <cstdlib>

#include "thread_pool_t.hpp"
#include "exception.cuh"
#include "slice_t.hpp"

#ifndef WARP_SZ
# define WARP_SZ 32
#endif

class gpu_t;
size_t ngpus();
const gpu_t& select_gpu(int id = 0);
const cudaDeviceProp& gpu_props(int id = 0);
const std::vector<const gpu_t*>& all_gpus();

class event_t {
    cudaEvent_t event;
public:
    event_t() : event(nullptr)
    {   CUDA_OK(cudaEventCreateWithFlags(&event, cudaEventDisableTiming));   }
    event_t(cudaStream_t stream) : event(nullptr)
    {
        CUDA_OK(cudaEventCreateWithFlags(&event, cudaEventDisableTiming));
        CUDA_OK(cudaEventRecord(event, stream));
    }
    ~event_t()
    {   if (event) (void)cudaEventDestroy(event);   }
    inline operator decltype(event)() const
    {   return event;   }

    inline void record(cudaStream_t stream)
    {   CUDA_OK(cudaEventRecord(event, stream));   }
    inline void wait(cudaStream_t stream)
    {   CUDA_OK(cudaStreamWaitEvent(stream, event, 0));   }
};

struct launch_params_t {
    dim3 gridDim, blockDim;
    size_t shared;

    launch_params_t(dim3 g, dim3 b, size_t sz = 0) : gridDim(g), blockDim(b), shared(sz) {}
    launch_params_t(int g, int b, size_t sz = 0) : gridDim(g>0 ? g : 0), blockDim(b), shared(sz) {}
    launch_params_t(unsigned int g, unsigned int b, size_t sz = 0) : gridDim(g), blockDim(b), shared(sz) {}

};

// HAZYNC_727_NOPOOL — hazync#631 / #727. sppark allocates with the stream-ordered allocator
// (cudaMallocAsync/cudaFreeAsync) without asking whether the device offers memory pools; where it
// does not, both return cudaErrorNotSupported. The answer is asked ONCE PER CUDA DEVICE and cached,
// so an allocation and its free can never disagree about which allocator made the pointer. Where
// pools exist (every Volta+ card the fleet runs) the calls are exactly the original ones.
// HAZYNC_NO_MEMPOOL=1 forces the synchronous fallback, so it can be exercised on a card with pools.
// ⚠ A FAILED QUERY IS NOT "NO POOLS" (review, 11-16): it keeps the stream-ordered allocator, which is
// what upstream always used, and says so loudly. Reading a failed query as "no pools" would silently
// move a Volta+ card off the allocator it has always run on.
inline bool hazync_pools_supported(int cuda_dev)
{
    static std::atomic<int> cache[64];             // 0 unknown, 1 pools, 2 no pools
    const bool cacheable = cuda_dev >= 0 && cuda_dev < 64;
    if (cacheable) {
        int c = cache[cuda_dev].load(std::memory_order_relaxed);
        if (c) return c == 1;
    }
    int v = 0;
    cudaError_t q = cudaDeviceGetAttribute(&v, cudaDevAttrMemoryPoolsSupported, cuda_dev);
    if (q != cudaSuccess) {
        (void)cudaGetLastError();                  // do not leave the failed query pending
        std::fprintf(stderr, "sppark: could not ask CUDA device %d whether it has memory pools (%s); "
                             "keeping the stream-ordered allocator (hazync#727)\n",
                     cuda_dev, cudaGetErrorString(q));
        v = 1;
    }
    bool forced = false;
    if (const char* s = std::getenv("HAZYNC_NO_MEMPOOL"))
        if (s[0] && s[0] != '0') { v = 0; forced = true; }
    int want = v ? 1 : 2;
    if (!cacheable)
        return want == 1;
    int expected = 0;
    if (!cache[cuda_dev].compare_exchange_strong(expected, want))
        return expected == 1;                      // another thread answered first: use its answer
    if (want == 2)
        std::fprintf(stderr, "sppark: CUDA device %d %s; using synchronous cudaMalloc/cudaFree "
                             "(hazync#727)\n", cuda_dev,
                     forced ? "told HAZYNC_NO_MEMPOOL" : "has no memory pools");
    return want == 1;
}

// The fallback's free (hazync#727): on the pointer's OWN device, after a DEVICE-wide sync.
// ⚠ Device-wide, not this stream: sppark hands one buffer to several streams of a device (the
// flipflop streams), and on an exception path a dev_ptr_t is destroyed before the gpu.sync() that
// would have waited for them -- cudaFree must not release memory another stream is still using.
// Only the no-pools fallback comes here; the pooled path is cudaFreeAsync, exactly as upstream.
inline cudaError_t hazync_sync_free(int cuda_dev, void* d_ptr) noexcept
{
    int cur = cuda_dev;
    (void)cudaGetDevice(&cur);
    if (cur != cuda_dev) (void)cudaSetDevice(cuda_dev);
    cudaError_t e = cudaDeviceSynchronize();
    cudaError_t f = cudaFree(d_ptr);
    if (cur != cuda_dev) (void)cudaSetDevice(cur);
    return e != cudaSuccess ? e : f;
}

class stream_t {
    cudaStream_t stream;
    const int gpu_id;
    int cuda_dev = 0;   // the CUDA device this stream belongs to (current at creation)
    bool pooled = true; // hazync#727: whether Dmalloc/Dfree use the stream-ordered allocator
public:
    stream_t(int id) : gpu_id(id)
    {   CUDA_OK(cudaStreamCreateWithFlags(&stream, cudaStreamNonBlocking));
        // ⚠ gpu_id is sppark's INDEX into its list of accepted cards, not a CUDA ordinal; a
        // stream belongs to the device current when it was created (gpus_t sets it first).
        CUDA_OK(cudaGetDevice(&cuda_dev));
        pooled = hazync_pools_supported(cuda_dev);
    }
    ~stream_t()
    {   (void)cudaStreamDestroy(stream);   }
    inline operator decltype(stream)() const    { return stream; }
    inline int id() const                       { return gpu_id; }
    inline operator int() const                 { return gpu_id; }
    inline bool pools() const                   { return pooled; }
    inline int device() const                   { return cuda_dev; }
    inline int sm_count() const
    {   return gpu_props(gpu_id).multiProcessorCount;   }

    inline void* Dmalloc(size_t sz) const
    {   void *d_ptr;
        if (pooled) {
            CUDA_OK(cudaMallocAsync(&d_ptr, sz, stream));
        } else {
            // Not stream-ordered: wait for queued work, and allocate on THIS stream's device.
            CUDA_OK(cudaStreamSynchronize(stream));
            int cur;
            CUDA_OK(cudaGetDevice(&cur));
            if (cur != cuda_dev) CUDA_OK(cudaSetDevice(cuda_dev));
            cudaError_t e = cudaMalloc(&d_ptr, sz);
            if (cur != cuda_dev) (void)cudaSetDevice(cur);
            CUDA_OK(e);
        }
        return d_ptr;
    }
    inline void Dfree(void* d_ptr) const
    {   if (pooled) {
            CUDA_OK(cudaFreeAsync(d_ptr, stream));
        } else {
            CUDA_OK(hazync_sync_free(cuda_dev, d_ptr));
        }
    }
    // The same pairing for destructors, which must not throw.
    inline void Dfree_nothrow(void* d_ptr) const
    {   if (pooled) {
            (void)cudaFreeAsync(d_ptr, stream);
        } else {
            (void)hazync_sync_free(cuda_dev, d_ptr);
        }
    }

    template<typename T>
    inline void bzero(T* dst, size_t nelems) const
    {   CUDA_OK(cudaMemsetAsync(dst, 0, nelems * sizeof(T), stream));   }

    template<typename T>
    inline void HtoD(T* dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   if (sz == sizeof(T))
            CUDA_OK(cudaMemcpyAsync(dst, src, nelems*sizeof(T),
                                    cudaMemcpyHostToDevice, stream));
        else
            CUDA_OK(cudaMemcpy2DAsync(dst, sizeof(T), src, sz,
                                      std::min(sizeof(T), sz), nelems,
                                      cudaMemcpyHostToDevice, stream));
    }
    template<typename T>
    inline void HtoD(T& dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   HtoD(&dst, src, nelems, sz);   }
    template<typename T, typename U>
    inline void HtoD(T& dst, const std::vector<U>& src,
                     size_t sz = sizeof(T)) const
    {   HtoD(&dst, &src[0], src.size(), sz);   }
    template<typename T, typename U>
    inline void HtoD(T* dst, const std::vector<U>& src,
                     size_t sz = sizeof(T)) const
    {   HtoD(dst, &src[0], src.size(), sz);   }
    template<typename T, typename U>
    inline void HtoD(T& dst, slice_t<U> src, size_t sz = sizeof(T)) const
    {   HtoD(&dst, &src[0], src.size(), sz);   }
    template<typename T, typename U>
    inline void HtoD(T* dst, slice_t<U> src, size_t sz = sizeof(T)) const
    {   HtoD(dst, &src[0], src.size(), sz);   }

    template<typename... Types>
    inline void launch_coop(void(*f)(Types...), dim3 gridDim, dim3 blockDim,
                                                size_t shared_sz,
                            Types... args) const
    {
#ifdef __NVCC__
        if (gpu_props(gpu_id).sharedMemPerBlock < shared_sz)
            CUDA_OK(cudaFuncSetAttribute(f, cudaFuncAttributeMaxDynamicSharedMemorySize, shared_sz));
#endif
        if (gridDim.x == 0 || blockDim.x == 0) {
            cudaFuncAttributes attr;
            CUDA_OK(cudaFuncGetAttributes(&attr, f));
            if (blockDim.x == 0) blockDim.x = attr.maxThreadsPerBlock;
            if (gridDim.x == 0)  gridDim.x = sm_count();
        }
        void* va_args[sizeof...(args)] = { &args... };
        CUDA_OK(cudaLaunchCooperativeKernel((const void*)f, gridDim, blockDim,
                                            va_args, shared_sz, stream));
    }
    template<typename... Types>
    inline void launch_coop(void(*f)(Types...), const launch_params_t& lps,
                            Types... args) const
    {   launch_coop(f, lps.gridDim, lps.blockDim, lps.shared, args...);   }

    template<typename T>
    inline void DtoH(T* dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   if (sz == sizeof(T))
            CUDA_OK(cudaMemcpyAsync(dst, src, nelems*sizeof(T),
                                    cudaMemcpyDeviceToHost, stream));
        else
            CUDA_OK(cudaMemcpy2DAsync(dst, sizeof(T), src, sz,
                                      std::min(sizeof(T), sz), nelems,
                                      cudaMemcpyDeviceToHost, stream));
    }
    template<typename T>
    inline void DtoH(T& dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   DtoH(&dst, src, nelems, sz);   }
    template<typename T>
    inline void DtoH(std::vector<T>& dst, const void* src,
                     size_t sz = sizeof(T)) const
    {   DtoH(&dst[0], src, dst.size(), sz);   }

    inline void sync() const
    {   CUDA_OK(cudaStreamSynchronize(stream));   }

    inline void notify(cudaHostFn_t cb, void* data)
    {   CUDA_OK(cudaLaunchHostFunc(stream, cb, data));   }

    template<class T>
    inline void notify(T& sema)
    {   notify([](void* s) { reinterpret_cast<T*>(s)->notify(); }, &sema);   }

    inline void record(cudaEvent_t event)
    {   CUDA_OK(cudaEventRecord(event, stream));   }
    inline void wait(cudaEvent_t event)
    {   CUDA_OK(cudaStreamWaitEvent(stream, event, 0));   }
};

class gpu_t {
public:
    static const size_t FLIP_FLOP = 3;
private:
    int gpu_id, cuda_id;
    cudaDeviceProp prop;
    size_t total_mem;
    mutable stream_t zero = {gpu_id};
    mutable stream_t flipflop[FLIP_FLOP] = {gpu_id, gpu_id, gpu_id};
    mutable thread_pool_t pool{"SPPARK_GPU_T_AFFINITY"};

public:
    gpu_t(int id, int real_id, const cudaDeviceProp& p)
    : gpu_id(id), cuda_id(real_id), prop(p)
    {   size_t freeMem;
        CUDA_OK(cudaMemGetInfo(&freeMem, &total_mem));
    }

    inline int cid() const                  { return cuda_id; }
    inline int id() const                   { return gpu_id; }
    inline operator int() const             { return gpu_id; }
    inline const auto& props() const        { return prop; }
    inline int sm_count() const             { return prop.multiProcessorCount; }
    inline void select() const              { (void)cudaSetDevice(cuda_id); }
    stream_t& operator[](size_t i) const    { return flipflop[i%FLIP_FLOP]; }
    inline operator stream_t&() const       { return zero; }
    inline operator cudaStream_t() const    { return zero; }

    inline size_t ncpus() const             { return pool.size(); }
    template<class Workable>
    inline void spawn(Workable work) const  { pool.spawn(work); }
    template<class Workable>
    inline void par_map(size_t num_items, size_t stride, Workable work,
                        size_t max_workers = 0) const
    {   pool.par_map(num_items, stride, work, max_workers);   }

    inline void* Dmalloc(size_t sz) const
    {   void *d_ptr = zero.Dmalloc(sz);
        zero.sync();
        return d_ptr;
    }
    inline void Dfree(void* d_ptr) const
    {   zero.Dfree(d_ptr);
        zero.sync();
    }

    template<typename T>
    inline void bzero(T* dst, size_t nelems) const
    {   zero.bzero(dst, nelems);   }

    template<typename T>
    inline void HtoD(T* dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   zero.HtoD(dst, src, nelems, sz);   }
    template<typename T>
    inline void HtoD(T& dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   HtoD(&dst, src, nelems, sz);   }
    template<typename T, typename U>
    inline void HtoD(T& dst, const std::vector<U>& src,
                     size_t sz = sizeof(T)) const
    {   HtoD(&dst, &src[0], src.size(), sz);   }

    template<typename... Types>
    inline void launch_coop(void(*f)(Types...), dim3 gridDim, dim3 blockDim,
                                                size_t shared_sz,
                            Types... args) const
    {   zero.launch_coop(f, gridDim, blockDim, shared_sz,
                         args...);
    }
    template<typename... Types>
    inline void launch_coop(void(*f)(Types...), const launch_params_t& lps,
                            Types... args) const
    {   zero.launch_coop(f, lps, args...);   }

    template<typename T>
    inline void DtoH(T* dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   zero.DtoH(dst, src, nelems, sz);   }
    template<typename T>
    inline void DtoH(T& dst, const void* src, size_t nelems,
                     size_t sz = sizeof(T)) const
    {   DtoH(&dst, src, nelems, sz);   }
    template<typename T>
    inline void DtoH(std::vector<T>& dst, const void* src,
                     size_t sz = sizeof(T)) const
    {   DtoH(&dst[0], src, dst.size(), sz);   }

    inline void sync() const
    {
        zero.sync();
        for (auto& f : flipflop)
            f.sync();
    }
};

template<typename T> class gpu_ptr_t {
    struct inner {
        T* ptr;
        std::atomic<size_t> ref_cnt;
        int real_id;
        inline inner(T* p) : ptr(p), ref_cnt(1)
        {   (void)cudaGetDevice(&real_id);   }
    };
    inner *ptr;
public:
    gpu_ptr_t() : ptr(nullptr)    {}
    gpu_ptr_t(T* p)               { ptr = new inner(p); }
    gpu_ptr_t(const gpu_ptr_t& r) { *this = r; }
    ~gpu_ptr_t()
    {
        if (ptr && ptr->ref_cnt.fetch_sub(1, std::memory_order_seq_cst) == 1) {
            int current_id;
            (void)cudaGetDevice(&current_id);
            if (current_id != ptr->real_id)
                (void)cudaSetDevice(ptr->real_id);
            (void)cudaFree(ptr->ptr);
            if (current_id != ptr->real_id)
                (void)cudaSetDevice(current_id);
            delete ptr;
        }
    }

    gpu_ptr_t& operator=(const gpu_ptr_t& r)
    {
        if (this != &r)
            (ptr = r.ptr)->ref_cnt.fetch_add(1, std::memory_order_relaxed);
        return *this;
    }
    gpu_ptr_t& operator=(gpu_ptr_t&& r) noexcept
    {
        if (this != &r) {
            ptr = r.ptr;
            r.ptr = nullptr;
        }
        return *this;
    }

    inline operator T*() const                  { return ptr->ptr; }

    // facilitate return by value through FFI, as gpu_ptr_t<T>::by_value.
    using by_value = struct { inner *ptr; };
    operator by_value() const
    {   ptr->ref_cnt.fetch_add(1, std::memory_order_relaxed); return {ptr};   }
    gpu_ptr_t(by_value v)   { ptr = v.ptr; }
};

// A simple way to allocate a temporary device pointer without having to
// care about freeing it.
template<typename T> class dev_ptr_t {
    T* d_ptr;
    size_t d_len_owned;
    cudaStream_t stream;
    // hazync#727: which allocator made d_ptr. The destructor used to choose by `stream` alone,
    // which was right only while every streamed allocation came from cudaMallocAsync.
    bool d_pooled = false;
    int d_dev = -1;     // the stream's CUDA device, for the fallback's free
public:
    dev_ptr_t(size_t nelems) : d_ptr(nullptr), d_len_owned(0), stream(nullptr)
    {
        if (nelems) {
            size_t n = (nelems+WARP_SZ-1) & ((size_t)0-WARP_SZ);
            CUDA_OK(cudaMalloc(&d_ptr, n * sizeof(T)));
            d_len_owned = (nelems << 1) | 1;
        }
    }
    dev_ptr_t(size_t nelems, const stream_t& s) : d_ptr(nullptr), stream(s)
    {
        if (nelems) {
            size_t n = (nelems+WARP_SZ-1) & ((size_t)0-WARP_SZ);
            d_pooled = s.pools();
            d_dev = s.device();
            d_ptr = static_cast<T*>(s.Dmalloc(n * sizeof(T)));
            d_len_owned = (nelems << 1) | 1;
        }
    }
    dev_ptr_t(T* ptr, size_t nelems = 0)
    : d_ptr(ptr), d_len_owned(nelems<<1), stream(nullptr) {}
    dev_ptr_t(const dev_ptr_t& r)
    : d_ptr(r.d_ptr), d_len_owned(r.d_len_owned & ((size_t)0-1) << 1), stream(nullptr) {}
    dev_ptr_t& operator=(const dev_ptr_t& r) = delete;
    ~dev_ptr_t()
    {
        if (d_ptr != nullptr && (d_len_owned&1)) {
            if (stream && d_pooled) (void)cudaFreeAsync((void*)d_ptr, stream);
            else if (stream)        (void)hazync_sync_free(d_dev, (void*)d_ptr);
            else                    (void)cudaFree((void*)d_ptr);   // upstream's stream-less path
        }
    }

    inline operator const T*() const            { return d_ptr; }
    inline operator T*() const                  { return d_ptr; }
    inline operator void*() const               { return (void*)d_ptr; }
    inline const T& operator[](size_t i) const  { return d_ptr[i]; }
    inline T& operator[](size_t i)              { return d_ptr[i]; }
    inline size_t size() const                  { return d_len_owned>>1; }
    inline T* data() const                      { return d_ptr; }
};

#if defined(_WIN32) && !defined(__HIP_DEVICE_COMPILE__)
# define SPPARK_FFI extern "C" __declspec(dllexport)
#else
# define SPPARK_FFI extern "C" __attribute__((visibility("default")))
#endif

SPPARK_FFI bool cuda_available();
SPPARK_FFI void drop_gpu_ptr_t(gpu_ptr_t<void>&);

#ifdef __clang__
# pragma clang diagnostic push
# pragma clang diagnostic ignored "-Wreturn-type-c-linkage"
#endif

SPPARK_FFI gpu_ptr_t<void>::by_value clone_gpu_ptr_t(const gpu_ptr_t<void>&);

#ifdef __clang__
# pragma clang diagnostic pop
#endif

template<typename T>
struct is_device_ptr {
    static const bool value{false};
};

template<typename T> struct is_device_ptr<gpu_ptr_t<T>> {
    static const bool value{true};
};
template<typename T> struct is_device_ptr<dev_ptr_t<T>> {
    static const bool value{true};
};

#endif
