use std::env;

#[cfg(unix)]
const DEV_NULL: &str = "/dev/null";
#[cfg(windows)]
const DEV_NULL: &str = "nul:";

fn common_flags(ccmd: &mut cc::Build) {
    ccmd.define("TAKE_RESPONSIBILITY_FOR_ERROR_MESSAGE", None);
    if let Some(include) = env::var_os("DEP_BLST_C_SRC") {
        ccmd.include(include);
    }
    if let Some(include) = env::var_os("DEP_SEMOLINA_C_INCLUDE") {
        ccmd.include(include);
    }
    if let Some(include) = env::var_os("DEP_SPPARK_ROOT") {
        ccmd.include(include);
    }
}

pub fn ccmd() -> cc::Build {
    let target = env::var("DEP_SPPARK_TARGET").unwrap_or_default();

    match target.as_str() {
        "cuda" => {
            fn is_cuda_flag_supported(nvcc: &cc::Build, flag: &str) -> bool {
                let out = nvcc
                    .get_compiler()
                    .to_command()
                    .arg(flag)
                    .args(["--dryrun", "-c", "-x", "cu", DEV_NULL])
                    .output()
                    .expect("impossible");

                out.status.success()
            }

            let mut nvcc = cc::Build::new();

            nvcc.cuda(true);
            if cfg!(feature = "build-poc") {
                nvcc.flag("-arch=native");
            } else {
                nvcc.flag("-gencode")
                    .flag("arch=compute_80,code=\"compute_80,sm_80\"")
                    .flag("-t0");
                if is_cuda_flag_supported(&nvcc, "-arch=sm_70") {
                    nvcc.flag("-gencode")
                        .flag("arch=compute_70,code=sm_70");
                } else if is_cuda_flag_supported(&nvcc, "-arch=sm_75") {
                    nvcc.flag("-gencode")
                        .flag("arch=compute_75,code=sm_75");
                }
                // ⛔⛔ WITHOUT THIS A PASCAL CARD HAS NO KERNEL IMAGE AT ALL, AND EVERY LAUNCH FAILS.
                // The lowest architecture above is sm_70. The embedded PTX is compute_80, and PTX
                // JIT only ever compiles FORWARD -- it cannot be lowered to an older architecture.
                // So on sm_61 the cubins do not match and the PTX cannot be used, and the first
                // kernel launch reports through sppark's next cudaGetLastError():
                //
                //   Failure during zk_shift: cudaGetLastError()@sppark/ntt/ntt.cuh:97
                //       failed: "operation not supported"
                //
                // ⚠ MEASURED 2026-10-06 on a real GTX 1050 Ti (sm_61), and it is why lowering the
                // DEVICE FILTER was necessary but not sufficient: HAZYNC_SPPARK_MIN_MAJOR=6 lets
                // sppark SELECT the card, and then there is nothing compiled for it to run. Two
                // earlier theories died here -- the device floor (hazync#631) and memory pools
                // (the nopool build reproduces this error byte for byte).
                //
                // ⚠ GUARDED, because CUDA 13 DROPPED Pascal: on a toolkit without sm_61 the flag
                // is simply not added, exactly as the sm_100/sm_120 probes above behave.
                if is_cuda_flag_supported(&nvcc, "-arch=sm_61") {
                    nvcc.flag("-gencode")
                        .flag("arch=compute_61,code=sm_61");
                }
                if is_cuda_flag_supported(&nvcc, "-arch=sm_100") {
                    nvcc.flag("-gencode")
                        .flag("arch=compute_100,code=compute_100");
                }
                if is_cuda_flag_supported(&nvcc, "-arch=sm_120") {
                    nvcc.flag("-gencode")
                        .flag("arch=compute_120,code=sm_120");
                }
            }
            #[cfg(not(target_env = "msvc"))]
            nvcc.flag("-Xcompiler").flag("-Wno-unused-function");
            common_flags(&mut nvcc);

            nvcc
        }
        "rocm" => {
            fn is_rocm_flag_supported(hipcc: &cc::Build, flag: &str) -> bool {
                let out = hipcc
                    .get_compiler()
                    .to_command()
                    .arg(flag)
                    .args(["-fsyntax-only", "-x", "hip", DEV_NULL])
                    .arg("-Wno-unused-command-line-argument")
                    .output()
                    .expect("impossible");

                out.status.success()
            }

            env::set_var("HIP_PLATFORM", "amd");

            let mut hipcc = cc::Build::new();

            hipcc.compiler(env::var("HIPCC").unwrap_or("hipcc".to_string()));
            hipcc.cpp(true);
            if cfg!(debug_assertions) {
                hipcc.opt_level(2); // no SGPR-s without optimization
            }
            if cfg!(feature = "build-poc") {
                hipcc.flag("--offload-arch=native");
            } else {
                // https://rocm.docs.amd.com/projects/install-on-linux/en/latest/reference/system-requirements.html
                if is_rocm_flag_supported(&hipcc, "--offload-arch=gfx1200") {
                    hipcc.flag("--offload-arch=gfx1201,gfx1200");
                }
                hipcc.flag("--offload-arch=gfx1102,gfx1101,gfx1100");
                hipcc.flag("--offload-arch=gfx1034,gfx1032,gfx1031,gfx1030");
                if is_rocm_flag_supported(&hipcc, "--offload-arch=gfx950") {
                    hipcc.flag("--offload-arch=gfx950");
                }
                hipcc.flag("--offload-arch=gfx942,gfx90a,gfx908");
                if let Ok(ncpus) = std::thread::available_parallelism() {
                    hipcc.flag_if_supported(format!("-parallel-jobs={}", ncpus).as_str());
                }
            }
            common_flags(&mut hipcc);
            hipcc.flag("-include").flag("util/cuda2hip.hpp");

            hipcc
        }
        _ => panic!("impossible DEP_SPPARK_TARGET={}", target),
    }
}
