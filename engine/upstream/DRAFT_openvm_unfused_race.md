# DRAFT, not posted (contacting OpenVM upstream is the user's call): stark-backend issue

**Title:** cuda-backend: frac_build_tree_layer_kernel's first virtual layer races across blocks (NonzeroRootSum when
the unfused bitrev path runs on more than one block)

**Version:** stark-backend v2.1.0 (a6b4761), crates/cuda-backend.

**What happens.** In `fractional.rs` (`fractional_sumcheck`), inputs with more than 1,024 leaves take the fused
`bit_rev_frac_ext_build_k2` kernel; smaller ones take the unfused path (`bit_rev_frac_ext`, then
`frac_build_tree_layer(..., apply_alpha = true)`, then `frac_add_alpha`). When the unfused path runs on a large input,
proving fails at the first segment with `LogupZerocheck(FractionalSumcheck(NonzeroRootSum))`.

**Why.** `frac_build_tree_layer_kernel<revert, apply_alpha = true>`, in its virtual first-layer branch
(`virtual_mode && layer_size == logical_len`, gkr.cu around lines 95-129), reads `layer[parent_start]` and
`layer[parent_start + 1]` (parent_start = bit-reversed idx, times 2) and then writes `layer[idx]` (and a right leaf
at idx + half) in place, ordered only by `__syncthreads()`. That orders the threads of one block. With up to 1,024
leaves it is one block; with more, a block can overwrite a slot that another block has not read yet.

**How we hit it.** On a GPU without 64 KB of shared memory per block (Pascal, e.g. GTX 1080 Ti: 48 KB), the fused
kernel's `cudaFuncSetAttribute(..., MaxDynamicSharedMemorySize, 64 KB)` fails (cudaErrorInvalidValue), so we gate the
fused kernel on `cudaDevAttrMaxSharedMemoryPerBlockOptin` and fall back to the unfused path, which then hits the race.

**Reproduced on a modern card** (so it is not Pascal-specific): an NVIDIA A40 with the unfused path forced on a large
proof (2,124-byte journal program; ~176 segments at the default segment size) fails at segment 0 after 42 s with
NonzeroRootSum; the fused path proves the same input.

**Fix we use** (patch below): the launcher takes a dedicated kernel for exactly that branch which reads every input
from a snapshot copy of the dense layer (`cudaMemcpyAsync` into a temporary buffer), so the values read are the ones a
single-block run reads, whatever the block order. With it, the same forced-unfused run proves and verifies, with a
journal byte-identical to the fused path's (756 s on the A40 for the whole input).

[patch: the gkr.cu hunks of engine/ci/stark-backend-pascal.patch; also optionally the device gate in fractional.rs
and ntt.rs, and the VPMM reservation fallback in vm_pool.rs (the 8 TB default reservation is refused by the driver on
a GTX 1080 Ti under Windows / WDDM), as separate issues]

Two related items we can file separately if useful: (1) the fused kernel's 64 KB requirement silently excludes
pre-Volta cards (a device check and fallback would make sm_6x work); (2) VPMM's default 8 TB virtual reservation is
refused on some drivers (a halving retry works).
