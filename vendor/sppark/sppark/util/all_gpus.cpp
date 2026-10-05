#include "gpu_t.cuh"
#include <cstdlib>
#include <cstdio>

#if defined(__NVCC__)
# define PROP_MAJOR_MIN 7   // Volta and forward
#elif defined(__HIPCC__)
# define PROP_MAJOR_MIN 9   // CDNA/RDNA
#else
# error "unknown platform"
#endif

// HAZYNC_631_PASCAL — hazync#631.
//
// ⛔⛔ THIS FILTER, NOT THE DRIVER, IS WHY A GTX 1050 Ti COULD NOT PROVE. Measured 2026-10-05: the
// card is enumerated by CUDA and listed by nvidia-smi, then DROPPED here because Pascal is
// compute 6.1 and PROP_MAJOR_MIN is 7. `gpus` comes back empty and select_gpu raises a synthetic
// `cudaErrorNoDevice` — "no CUDA-capable device is detected" — on a machine that plainly has one.
//
// ⭐ AND THE KERNELS IN THIS BUILD ARE COMPILED FOR THAT CARD. windows-prove.yml pins CUDA 12.x
// precisely because CUDA 13 dropped Pascal, and passes
// `-gencode arch=compute_61,code=sm_61` plus PTX for compute_61. So the binary has code the card
// can run; only this runtime check rejects it.
//
// ⚠ TWO THINGS ARE DELIBERATE HERE:
//
//   1. THE DEFAULT IS UNCHANGED. Upstream chose 7 and may have reasons this project has not hit.
//      The floor moves only when HAZYNC_SPPARK_MIN_MAJOR is set, so every existing machine behaves
//      exactly as before and no proof is produced differently by accident.
//   2. IT REPORTS EVERY DEVICE AND WHY IT WAS REJECTED. The first attempt at this cost a 177 MB
//      download, a special build and most of a morning, to learn one number. `cooperativeLaunch` is
//      the OTHER half of this condition and nothing prints it, so lowering the floor alone could
//      have failed again for a second invisible reason.
//
// ⛔ A PROOF FROM A LOWERED FLOOR IS NOT TRUSTED BLIND: `host prove-block` verifies the receipt it
// produces, and the coordinator re-verifies every submission. A card that computes wrongly fails
// verification; it does not get onto the board.
static int hazync_min_major()
{
    static int cached = -1;
    if (cached >= 0) return cached;
    cached = PROP_MAJOR_MIN;
    if (const char* s = std::getenv("HAZYNC_SPPARK_MIN_MAJOR")) {
        int v = std::atoi(s);
        if (v >= 1 && v <= 99) cached = v;
    }
    return cached;
}

class gpus_t {
    std::vector<const gpu_t*> gpus;
public:
    gpus_t()
    {
        int n;
        const int min_major = hazync_min_major();
        const bool report = std::getenv("HAZYNC_GPU_REPORT") != nullptr
                         || min_major != PROP_MAJOR_MIN;
        cudaError_t cnt = cudaGetDeviceCount(&n);
        if (cnt != cudaSuccess) {
            if (report)
                std::fprintf(stderr, "\nGPU SCAN: cudaGetDeviceCount failed: %s\n",
                             cudaGetErrorString(cnt));
            return;
        }
        if (report)
            std::fprintf(stderr, "\nGPU SCAN: %d device(s), need major >= %d and "
                                 "cooperativeLaunch\n", n, min_major);
        for (int id = 0; id < n; id++) {
            cudaDeviceProp prop;
            if (cudaGetDeviceProperties(&prop, id) != cudaSuccess) {
                if (report)
                    std::fprintf(stderr, "  [%d] properties unreadable — skipped\n", id);
                continue;
            }
            const bool ok_major = prop.major >= min_major;
            const bool ok_coop  = prop.cooperativeLaunch != 0;
            if (report)
                std::fprintf(stderr, "  [%d] %s  compute %d.%d  cooperativeLaunch=%d  -> %s%s%s\n",
                             id, prop.name, prop.major, prop.minor, prop.cooperativeLaunch,
                             (ok_major && ok_coop) ? "USED" : "SKIPPED",
                             ok_major ? "" : " (compute too low)",
                             ok_coop ? "" : " (no cooperative launch)");
            if (ok_major && ok_coop) {
                (void)cudaSetDevice(id);
                gpus.push_back(new gpu_t(gpus.size(), id, prop));
            }
        }
        if (report)
            std::fprintf(stderr, "GPU SCAN: %zu device(s) usable\n", gpus.size());
        (void)cudaSetDevice(0);
    }
    ~gpus_t()
    {   for (auto* ptr: gpus) delete ptr;   }

    static const auto& all()
    {
        static gpus_t all_gpus;
        return all_gpus.gpus;
    }
};

const gpu_t& select_gpu(int id)
{
    auto& gpus = gpus_t::all();
    if (gpus.size() == 0)
        CUDA_OK(cudaErrorNoDevice);
    if (id == -1) {
        int cuda_id;
        CUDA_OK(cudaGetDevice(&cuda_id));
        for (auto* gpu: gpus)
           if (gpu->cid() == cuda_id) return *gpu;
        id = 0;
    }
    auto* gpu = gpus[id];
    gpu->select();
    return *gpu;
}

const cudaDeviceProp& gpu_props(int id)
{   return gpus_t::all()[id]->props();   }

size_t ngpus()
{   return gpus_t::all().size();   }

const std::vector<const gpu_t*>& all_gpus()
{   return gpus_t::all();   }

SPPARK_FFI bool cuda_available()
{   return gpus_t::all().size() != 0;   }

SPPARK_FFI void drop_gpu_ptr_t(gpu_ptr_t<void>& ref)
{   ref.~gpu_ptr_t();   }

#ifdef __clang__
# pragma clang diagnostic push
# pragma clang diagnostic ignored "-Wreturn-type-c-linkage"
#endif

SPPARK_FFI gpu_ptr_t<void>::by_value clone_gpu_ptr_t(const gpu_ptr_t<void>& rhs)
{   return rhs;   }

#ifdef __clang__
# pragma clang diagnostic pop
#endif

#ifdef TAKE_RESPONSIBILITY_FOR_ERROR_MESSAGE
SPPARK_FFI void drop_error_message(char *ptr)
{   free(ptr);   }
#endif
