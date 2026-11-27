# R1-4: C heap and node format 2 (reviewer report, 27.11.2026, as given)

Nothing that breaks soundness in either scope; residual risks and liveness-only issues. Read-only. No engine guest ELF
on the machine: symbols checked on a RISC Zero libzync ELF of 08.11 (before c_heap), a stand-in for the C/C++ side only.

Scope A, C heap (prover/methods/guest/src/c_heap.rs, engine/guest/build.rs)
- A1 LOW-MEDIUM (residual risk; high confidence in the mechanism). Recycling turns a latent use-after-free / double free
  in the C/C++ code from harmless (bump allocator) into live aliasing. The double-free check (c_heap.rs:113) is
  incomplete: free(a); b = malloc(same class) gives b == a; a second free(a) passes the LIVE check and returns b's block
  while b is in use; the next malloc of that class aliases b. Detecting a free of a foreign pointer is probabilistic
  (word at ptr-4 vs LIVE_MARK). Main exposure: Hazync's own C++ (patches/libzync/0101, verify_input.cpp), written and
  tested under a bump allocator; e.g. cached_tx returns *e.tx, which hz_cache_reset destroys. No actual use-after-free
  found; the heap-poison runs are the tool to look.
- A2 LOW. The block header is trusted: an underflow overwriting word0 (class) or word1 (capacity) breaks the heap
  (larger class / capacity in realloc c_heap.rs:144 overlaps the neighbour; out-of-range class panics). Needs an
  existing out-of-bounds write first.
- A3 LOW (liveness; medium). Two heaps: newlib's _malloc_r / _free_r / _realloc_r on the 1 MiB static _heap
  (cshims.c:84), ours on the bump heap; disjoint. A newlib chunk into our free aborts (size word != 0x4c495645); one of
  ours into _free_r would corrupt. In the 08.11 ELF only newlib's stdio, dtoa, tzset call the _r functions, paired. An
  over-aligned operator new would go through newlib's aligned_alloc then our free (abort); no alignas > 16 on heap types
  in the compiled Core TUs. Re-check on the engine ELF.
- A4 INFO. cshims.c:80-81 says "nothing in the guest catches bad_alloc": wrong, EvalScript has catch (...)
  (interpreter.cpp:1254) -> SCRIPT_ERR_UNKNOWN_ERROR. Our malloc returns NULL only on size overflow / Layout failure
  (> ~2 GiB): a valid script would fail (liveness).
- A5 INFO. realloc(p, 0) returns NULL without freeing p (a leak, not aliasing).
- Sound: class arithmetic (small 2..128, large 129..204, N_CLASSES 205; cap >= total; capacity fixed by class; the
  free-list link in the header, so a UAF write to user data cannot redirect the list; overflow checked in malloc and
  calloc; realloc copies min(room, size)); 16-byte alignment; exhaustion (OpenVM v2.1.0 bump allocator
  platform/src/memory.rs:58: checked_add, <= 2^29, then terminate::<1>; never wraps; verify-core checks the exit code;
  stack below the program text, STACK_TOP 0x200400); the Rust allocator never reuses and never gets C-heap memory
  (startup_check asserts it); the legacy sighash cache key cleared on vin and vout buffers in free; build.rs:82 refuses a
  Core tree without the hz_lsh_key hunk.

Scope B, node format 2: no soundness issue (high confidence).
- Domain separation: coin log IV_NODE[l] per level, IV_SPENT for spent leaves, unspent leaf a full tagged SHA-256; trie
  separate IVs from the same table (50 inner, ttree.rs:233 / trie_store.rs:749; 51 coins, ttree.rs:173; 53/54
  addresses, ttree.rs:183-187); block forest format-independent (facts.rs:92). Format 1 is format 2 with each output
  word byte-swapped over the same compression: a wrong-format reader only rejects; a cross-format path needs a preimage.
- B1 INFO. IV_NODE[50..54] shared between coin-log levels >= 50 and the trie's roles; member() (facts.rs:58-64) allows
  64 levels but the root set must match the committed leaf count; level 50 needs 2^50 coins: unreachable.
- B2 INFO. A coin hashed with counts=false equals the same coin with counts=true and value 0 (IV 51, zero words);
  harmless while counts follows the profile.
- Format state: the guest hard-wires format 2 (coinlog_ovm.py; main.rs:1527 sets bit 7); recursion, handover and join
  assert prev.profile == chain_profile() (main.rs:2288, 2293, 2568). Every swiftsync-verify reader calls
  use_format_of(cs.profile) before any path (lib.rs:308, 526, 786/809, 933/943, 1070/1109; the statement zAPP via
  check_trie_fact; liveness lib.rs:126/234; deposit lib.rs:468), then the profile is checked against the image (:532).
  swiftsync-verify enables libzync `tls`, so the format is per thread in FFI and wasm too.
- B3 LOW (liveness). set_node_format is never reset and producers never set it (zync_path.rs, zync_serve.rs, liveness
  make_liveness / prove, statement draft, make_trie_fact, a TrieStore updated in a thread): each computes in whatever
  format the last verify on that thread left. Wrong format = paths that fail verification, never a false accept. A
  libzync build with neither std nor tls/reader falls back to one process-wide AtomicBool (shared only in threaded wasm).
