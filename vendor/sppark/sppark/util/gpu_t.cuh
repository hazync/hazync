// Copyright Supranational LLC
// Licensed under the Apache License, Version 2.0, see LICENSE for details.
// SPDX-License-Identifier: Apache-2.0

#ifndef __SPPARK_UTIL_GPU_T_CUH__
#define __SPPARK_UTIL_GPU_T_CUH__

#if defined(__NVCC__) && !defined(__CUDACC__)
# include <cuda_runtime.h>
#endif

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

class stream_t {
    cudaStream_t stream;
    const int gpu_id;
public:
    stream_t(int id) : gpu_id(id)
    {   CUDA_OK(cudaStreamCreateWithFlags(&stream, cudaStreamNonBlocking));   }
    ~stream_t()
    {   (void)cudaStreamDestroy(stream);   }
    inline operator decltype(stream)() const    { return stream; }
    inline int id() const                       { return gpu_id; }
    inline operator int() const                 { return gpu_id; }
    inline int sm_count() const
    {   return gpu_props(gpu_id).multiProcessorCount;   }

    // HAZYNC_631_NOPOOL — hazync#631.
    //
    // ⛔⛔ cudaMallocAsync IS CALLED HERE UNCONDITIONALLY, AND IT IS NOT UNIVERSALLY SUPPORTED.
    // The stream-ordered allocator needs the driver to offer MEMORY POOLS, and when it does not,
    // cudaMallocAsync returns exactly `cudaErrorNotSupported` — "operation not supported". sppark
    // never asks whether pools exist; it just calls it.
    //
    // 📏 Measured on Windows 2026-10-05, GTX 1050 Ti, once the compute-capability floor was
    // lowered: the device was accepted, and proving then died with
    //
    //     CUDA ERROR: cudaGetLastError()@sppark/ntt/ntt.cuh:97 failed: "operation not supported"
    //
    // ⚠ ntt.cuh:97 is a check AFTER an innocent kernel launch. cudaGetLastError reports the first
    // error since the last check, so a failed allocation earlier in the same stream surfaces
    // there — which is why the line number pointed at a kernel with nothing wrong with it.
    //
    // ⭐ IF THAT IS THE CAUSE, IT IS A PLATFORM LIMITATION AND NOT A PASCAL ONE: any card on a
    // driver without pool support fails the same way. This makes that answerable and survivable.
    //
    // ⛔ THE PAIRING MUST NOT BE MIXED. A pointer from cudaMallocAsync must be freed by
    // cudaFreeAsync and a pointer from cudaMalloc by cudaFree, so the decision is made ONCE per
    // device and cached — not re-tested per call, where a flapping answer would free a pointer
    // with the wrong allocator.
    //
    // ⚠ The synchronous path SYNCHRONISES FIRST. cudaMalloc/cudaFree are not stream-ordered, so
    // issuing one while earlier work is still queued on this stream would free memory the GPU is
    // still reading. The async allocator exists precisely to avoid that stall; where it is
    // unavailable, correctness beats the stall.
    static bool hazync_pools_supported(int gpu_id)
    {
        static int cached = -1;
        if (cached < 0) {
            int v = 0;
            if (cudaDeviceGetAttribute(&v, cudaDevAttrMemoryPoolsSupported, gpu_id) != cudaSuccess)
                v = 0;
            if (const char* s = std::getenv("HAZYNC_NO_MEMPOOL"))
                if (s[0] && s[0] != '0') v = 0;      // force the fallback, to test it anywhere
            cached = v ? 1 : 0;
            if (!cached)
                std::fprintf(stderr, "\nGPU SCAN: memory pools NOT supported — using synchronous "
                                     "cudaMalloc/cudaFree (hazync#631)\n");
        }
        return cached == 1;
    }

    inline void* Dmalloc(size_t sz) const
    {   void *d_ptr;
        if (hazync_pools_supported(gpu_id)) {
            CUDA_OK(cudaMallocAsync(&d_ptr, sz, stream));
        } else {
            CUDA_OK(cudaStreamSynchronize(stream));
            CUDA_OK(cudaMalloc(&d_ptr, sz));
        }
        return d_ptr;
    }
    inline void Dfree(void* d_ptr) const
    {   if (hazync_pools_supported(gpu_id)) {
            CUDA_OK(cudaFreeAsync(d_ptr, stream));
        } else {
            CUDA_OK(cudaStreamSynchronize(stream));
            CUDA_OK(cudaFree(d_ptr));
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
    // ⛔⛔ WHICH ALLOCATOR MADE THIS POINTER. The destructor below used to choose by `stream`
    // alone, which was safe only while every streamed allocation came from cudaMallocAsync. With
    // a synchronous fallback for drivers without memory pools (hazync#631), choosing by `stream`
    // would free a cudaMalloc pointer with cudaFreeAsync — mixing the pair, which is undefined.
    // ⚠ Per OBJECT, not a global: a multi-GPU box can have pools on one device and not another,
    // and this fleet runs many cards.
    bool d_pooled = false;
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
            // HAZYNC_631_NOPOOL — the SECOND unconditional cudaMallocAsync in this file. Fixing
            // only stream_t::Dmalloc would have left this one failing identically, on a driver
            // without memory pools. ⚠ `stream` is recorded here and the destructor frees by it, so
            // the two must agree about which allocator was used: both ask the same cached answer.
            d_pooled = stream_t::hazync_pools_supported((int)s);
            if (d_pooled) {
                CUDA_OK(cudaMallocAsync(&d_ptr, n * sizeof(T), s));
            } else {
                CUDA_OK(cudaStreamSynchronize(s));
                CUDA_OK(cudaMalloc(&d_ptr, n * sizeof(T)));
            }
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
            // ⛔ PAIRED WITH WHAT ALLOCATED IT, not with whether a stream exists.
            if (stream && d_pooled) (void)cudaFreeAsync((void*)d_ptr, stream);
            else {
                if (stream) (void)cudaStreamSynchronize(stream);
                (void)cudaFree((void*)d_ptr);
            }
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
