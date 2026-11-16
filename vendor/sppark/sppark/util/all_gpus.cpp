#include "gpu_t.cuh"
#include <cstdio>
#include <cstdlib>

#if defined(__NVCC__)
# define PROP_MAJOR_MIN 7   // Volta and forward
#elif defined(__HIPCC__)
# define PROP_MAJOR_MIN 9   // CDNA/RDNA
#else
# error "unknown platform"
#endif

// HAZYNC_727_FLOOR — hazync#631 / #727. sppark keeps only cards with major >= PROP_MAJOR_MIN (7:
// Volta and forward), so a Pascal card (GTX 10-series, compute 6.1) is dropped here and select_gpu
// raises a synthetic cudaErrorNoDevice on a machine that plainly has one. The Windows build carries
// sm_61 code for exactly that card. HAZYNC_SPPARK_MIN_MAJOR lowers the floor -- the Windows app sets it
// only when its "try my GPU even if it is below the supported floor" box is ticked AND the worker's
// card is below 7 (tools/win-gui/hazync_gui.py, _spawn). Unset, or anything outside 6..99, keeps 7:
// every existing machine behaves exactly as before. The cooperativeLaunch requirement is unchanged.
// ⚠ The floor is PROCESS-WIDE: on a machine with an old and a new card it admits both, so pair it with
// CUDA_VISIBLE_DEVICES naming the old card -- the app does (one worker per card, supervisor.worker_env).
// ⚠ Below 6 is REFUSED, not honoured: the oldest code any build carries is sm_61 + compute_61 PTX,
// so a 5.x card would be admitted only to fail with "no kernel image". (6 admits a 6.0 card, which
// has no image either; the env var carries a major version only, and the app sets it to the card's.)
// A receipt from a lowered floor is not trusted blind: the host verifies what it proves and the
// coordinator re-verifies every submission.
static int hazync_min_major()
{
    int floor = PROP_MAJOR_MIN;
    if (const char* s = std::getenv("HAZYNC_SPPARK_MIN_MAJOR")) {
        char* end = nullptr;
        long v = std::strtol(s, &end, 10);
        if (end != s && *end == '\0' && v >= 6 && v <= 99)
            floor = (int)v;
        else
            std::fprintf(stderr, "sppark: HAZYNC_SPPARK_MIN_MAJOR=%s ignored (want 6..99); "
                                 "keeping %d (hazync#727)\n", s, floor);
    }
    return floor;
}

class gpus_t {
    std::vector<const gpu_t*> gpus;
public:
    gpus_t()
    {
        int n;
        if (cudaGetDeviceCount(&n) != cudaSuccess)
            return;
        const int min_major = hazync_min_major();
        for (int id = 0; id < n; id++) {
            cudaDeviceProp prop;
            if (cudaGetDeviceProperties(&prop, id) == cudaSuccess &&
                prop.major >= min_major && prop.cooperativeLaunch) {
                (void)cudaSetDevice(id);
                gpus.push_back(new gpu_t(gpus.size(), id, prop));
            }
        }
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
