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

use std::env;

use risc0_build_kernel::{KernelBuild, KernelType};

fn main() {
    if env::var("CARGO_FEATURE_CUDA").is_ok() {
        build_cuda_kernels();
    }
}

fn build_cuda_kernels() {
    let mut build = KernelBuild::new(KernelType::Cuda);
    if env::var("CARGO_FEATURE_SETUP").is_ok() {
        build.flag("-DSRS_READ_COEFFS");
    }
    build
        .files(["kernels/cuda/ffi.cu"])
        .deps(["kernels/cuda"])
        .flag("-D__ADX__")
        .include(watch_dep("DEP_BLST_C_SRC"))
        .include(watch_dep("DEP_SPPARK_ROOT"))
        .compile("risc0_groth16_cuda");

    println!("cargo:rustc-link-lib=static=blst");
}

// HAZYNC_RERUN_ON_HEADERS — every include directory a `links` dependency hands this script (DEP_*_ROOT) is
// watched, so a header change there re-runs this script and recompiles its kernels even on a warm target.
// ⛔ Without it this script watched only its own kernels/, and the v0.26.1/v0.27.0 Linux CUDA host linked
// objects compiled against two layouts of sppark's stream_t (gpu_t.cuh, #727) and segfaulted on its first
// GPU step. cargo scans a watched directory recursively by mtime; an absolute path outside the package is
// allowed. scripts/test-build-rerun.sh checks no build.rs here reads a DEP_* path any other way.
fn watch_dep(var: &str) -> String {
    let p = env::var(var).unwrap();
    println!("cargo:rerun-if-changed={p}");
    p
}
