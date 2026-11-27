# Our changes to OpenVM and stark-backend (ROADMAP 4.3; 27.11.2026)

Every patch Hazync applies to OpenVM v2.1.0 (05f3b73e) or stark-backend v2.1.0 (a6b4761a), what it is for, whether it
touches anything a proof's soundness rests on, and whether it is fit to offer upstream. Patches are applied only by
engine/scripts/openvm_src.sh, to the pinned commits, never to a cargo checkout. (The patches to Bitcoin Core and
libsecp256k1 under engine/guest/ are Hazync's guest code, not OpenVM's: out of scope here, reviewed in 4.2.)

**The soundness question first:** no patch changes an AIR, a constraint, a key, the transcript or a verifier check.
Three only make a dependency optional (default unchanged); one swaps a container type in a crate the WASM verifier
compiles but never runs; two let the CUDA prover compile under MSVC. A prover-side change can make a proof fail,
never make a false one verify: the verifier and the constraints are unpatched, and the readers pin the key. Builds:
the native Linux prover uses NO patch (hzengine, verify-core and the guest manifests have no [patch] section;
checked 27.11); only an image built with the `ovm-xver` feature (the handover guest) takes the patched tree, through
engine/guest/assemble.sh.

| patch | lines changed | used by | what | soundness path | upstream |
|---|---|---|---|---|---|
| handover/openvm-verify-nozstd.patch (openvm crates/verify) | 13 | WASM verifier, handover guest | zstd becomes an optional feature `compressed`, ON by default; without it only the plain proof encoding (what HZE1 carries) is compiled | no: removes a decoder, adds no code | ready: a feature flag, default unchanged |
| handover/stark-sdk-nologging.patch (stark-backend crates/stark-sdk) | 23 | WASM verifier, handover guest | the logging/metrics stack becomes an optional feature `logging`, ON by default (quanta needs a clock a zkVM guest lacks) | no: logging only | ready: a feature flag, default unchanged |
| wasm-verify/openvm-abi-stable.patch (openvm crates/vm) | 3 | WASM verifier | `abi_stable::RVec` -> `Vec` in the metered executor's memory context (abi_stable does not build for wasm32); the verifier compiles this crate but never executes | no: executor-side type, not run by a verifier | needs work: upstream would want abi_stable behind a feature, or verify-stark-host not to pull crates/vm |
| ci/openvm-msvc.patch (openvm: vm, recursion, rv32im, bigint, poseidon2-air, primitives CUDA) | 101 | Windows CUDA prover (2.9) | MSVC/nvcc: GCC-only warning flag gated on target_env; compound literals -> `std::array` (also under GCC: same values); GNU `?:` -> `a ? a : b` (also under GCC); a preprocessor re-scan for MSVC's `__VA_ARGS__`; zero-length sbox arrays specialised under `_MSC_VER` with the same column offsets; CUB sum functors under `_MSC_VER` | prover only (trace generation, CUDA): a wrong offset gives a failing proof, not a false one; to check on Windows that proofs verify (2.9) | ready as a "build on MSVC" PR once proved on Windows |
| ci/stark-backend-msvc.patch (stark-backend cuda-common) | 12 | Windows CUDA prover | a C++20 designated initializer -> member assignments (MSVC under --std=c++17) | prover only (device memory setup) | ready |
| handover/v2.0.2/openvm-v2.0.2-links.patch, stark-v2.0.1-links.patch | 2 + 2 | handover rehearsal (step 3) only | renames the CUDA `links` names so a second OpenVM version can sit in one crate graph | no: build metadata | not for upstream (our two-version build) |
| handover/v2.0.2/openvm-v2.0.2-verify-nozstd.patch | 7 | handover rehearsal only | nozstd for the v2.0.2 copy | no | upstream has v2.1.0's equivalent need |
| handover/v2.0.2/engine-on-v2.0.2.patch | 23 | handover rehearsal only | OUR code (engine/host, verify-core) adapted to v2.0.2's API, not an OpenVM patch | - | - |

## Smaller

- The three verifier-side patches (nozstd, nologging, abi-stable: 39 lines) are what the shipped WASM verifier and the
  handover guest need. Offered upstream and merged, they go away; until then they stay this small.
- abi-stable cannot go on our side: `cargo tree -i openvm-circuit` on verify-core shows openvm-verify-stark-host
  itself depends on openvm-circuit (crates/vm), so the verifier graph has it whatever we do. Upstream: abi_stable behind
  a feature, or verify-stark-host without crates/vm.
- The MSVC pair is only for Windows proving (2.9, parked by the user): not on any shipped Linux build.

## Upstream offers

Drafts only. Contacting OpenVM is the user's call (feedback: contacting external projects). In order of readiness:
nozstd (feature `compressed`), nologging (feature `logging`), the MSVC build fixes (after 2.9 proves on Windows), and
an issue for abi_stable on wasm32.
