// hazync#727 negative-control source for scripts/gpu_code_archs.py --control.
// The operation in rv32im's LookupTables::lookupDelta, at a scope chosen at build time. The four
// fatbins beside this file were built from it with CUDA 12.9.1 (nvcc 12.9.86) on Linux:
//
//   F="-fatbin -Wno-deprecated-gpu-targets -O2 -std=c++17"
//   SYS=-DHZ_SCOPE=::cuda::thread_scope_system  DEV=-DHZ_SCOPE=::cuda::thread_scope_device
//   BOTH="-gencode=arch=compute_61,code=sm_61 -gencode=arch=compute_61,code=compute_61"
//   nvcc $F $BOTH -Xfatbin -compress-all $SYS -o sys.fatbin      atom_scope.cu  # must be REFUSED
//   nvcc $F $BOTH -Xfatbin -compress-all $DEV -o dev.fatbin      atom_scope.cu  # must PASS
//   nvcc $F -gencode=arch=compute_61,code=sm_61 -Xfatbin -compress=false $SYS -o sys_raw.fatbin atom_scope.cu
//   nvcc $F -gencode=arch=compute_61,code=compute_61 -Xfatbin -compress-all $SYS -o sys_ptx.fatbin atom_scope.cu
//
// `cuobjdump -elf sys.fatbin` shows EIATTR_ATOM_SYS_INSTR_OFFSETS for `count`; dev.fatbin has none.
#include <cuda/atomic>
__global__ void count(::cuda::atomic<unsigned, HZ_SCOPE>* c) { (*c)++; }
