# Reclaiming memory in the guest (#794 / FEASIBILITY Q7), 26.11.2026

OpenVM's guest memory is 512 MiB at most (pointer_max_bits <= F::bits() - 2 = 29); the engine at 2^29 could not prove
tap_small (one tapscript input; RISC Zero's 3 GiB guest can). The guest's allocators never freed memory.

**Two designs, measured on block 976,379 (CPU cost, `hzengine execute`, against the bump-allocator image keys-f2):**

| | chunk_00 | chunk_01 | chunk_02 | chunk_03 | chunk_04 | trie | journals |
|---|---|---|---|---|---|---|---|
| OpenVM's `heap-embedded-alloc` for everything (linked-list first fit; prototype 00e88ec0, reverted) | +116% | +52% | +204% | +90% | +45% | +0.5% | identical |
| **#794's C-heap recycling** (fix/guest-heap-reclaim, cherry-picked: per-size-class free lists for C/C++ only, Rust stays bump; 4cb9e0c1) | **+1.0%** | **+0.9%** | **+1.1%** | **+1.1%** | **+1.1%** | **+0.0%** | **identical to RISC Zero's** |

The C-heap design (reviewed in #794's round: the legacy sighash cache's key is cleared when its buffers are freed,
start-up checks that Rust's allocator never reuses and the C heap does, double-free detection) costs about 1% and is
the one adopted. The address-keyed caches stay sound because their keys point at Rust (bump) buffers, which are
never reused; Core's per-opcode allocations, the bulk of a script's memory traffic, are recycled.
