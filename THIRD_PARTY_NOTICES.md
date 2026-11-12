# Third-party notices

Hazync is MIT-licensed (see [`LICENSE`](LICENSE)). It builds on, and in some cases compiles in,
third-party components under their own licences. This file records them.

## Compiled into the zkVM guest

The patch sets below are the ones Hazync v2's published images (v0.24.0) are built with, as applied by
`scripts/libzync-base.sh`; Hazync v1's CORE build applied a subset.

- **Bitcoin Core** — MIT. `bitcoin/bitcoin` tag `v28.0`. The guest compiles Core's real consensus
  sources (`interpreter.cpp`, `pubkey.cpp`, sighash, serialization) with patches that change no
  consensus rule: `patches/0001` (an ILP32 `Serialize` overload), `patches/0002`, `0009` and `0010` (SHA-256
  compression routed through the RISC Zero accelerator, byte-identical output), and
  `patches/libzync/0101` (libzync's levers, each a cheaper way to compute the same bytes: SHA-256 state in the
  accelerator's layout, lazy txid/wtxid, memos of parsed keys and HASH160, legacy sighash from preserialised
  slices). Copyright (c) 2009-present The Bitcoin Core developers.
- **libsecp256k1** — MIT. `bitcoin-core/secp256k1` tag `v0.5.1`, compiled for real ECDSA and Schnorr
  verification. Copyright (c) 2013 Pieter Wuille and contributors. **Modified:** `patches/0012` (selects a
  coprocessor-backed field backend at libsecp's field-backend interface; the backend sources are
  `prover/methods/guest/field_bigint2*.h`) and `patches/0013` (recovers a pubkey's Y from a witness hint that
  libsecp's own field arithmetic checks before accepting), both since v0.21.0; and in Hazync v2 also `0005`,
  `0006`, `0008` (ECDSA and Schnorr group arithmetic and scalar inversion through the coprocessor),
  `patches/libzync/0102` (batch-verification hooks) and `0103` (SHA-256 through the accelerator).

## Build / proving stack (linked, not part of the consensus path)

- **RISC Zero (risc0)** zkVM, `risc0-zkvm` `=3.1.0` (circuits 4.1.0; since v0.24.0, before that 3.0.5) /
  `risc0-build` `=3.0.7`, and the rzup
  cross-toolchain — Apache-2.0. The `prover/` crate was scaffolded from the risc0 project template and
  carries an additional Apache-2.0 notice at [`prover/LICENSE`](prover/LICENSE) covering that
  risc0-derived build scaffolding. Copyright (c) RISC Zero, Inc.
- **RustCrypto `sha2`** — MIT/Apache-2.0, pinned to an immutable commit, routed through the risc0
  SHA-256 accelerator (byte-identical output).

## Vendored and modified (Apache-2.0)

Copied into `vendor/` and wired in through `[patch.crates-io]` in `prover/Cargo.toml`. All but `sppark` are RISC
Zero crates, Copyright (c) RISC Zero, Inc. Each is upstream's published crate except for the changes stated here,
which are marked in the source (`hazync patch`, `hazync#…`). Every change is prover-side: the image ids, the circuit
and the verifier are unchanged.

- **`vendor/risc0-zkvm`** — `risc0-zkvm` 3.1.0. Modified in `src/host/server/prove/mod.rs` and
  `src/host/server/prove/prover_impl.rs` only: an API to assemble a receipt from segment receipts
  proved on other machines (segment distribution, hazync#148, #153, #161), and a balanced join tree
  in place of the linear fold. Host-side proving only.
- **`vendor/risc0-circuit-rv32im-sys`** — `risc0-circuit-rv32im-sys` 4.1.0. Modified in `build.rs`
  only (marked `HAZYNC_631_MSVC_FLAGS`): the CUDA flags MSVC needs to build on Windows (hazync#631).
  The hazync#119 accum-pass fix we used to carry is upstream since 4.1.0. Prover-only.
- **`vendor/risc0-circuit-keccak`** — 4.1.0. Modified in `build.rs` only (`HAZYNC_WINDOWS_PATH_ESCAPE`):
  backslashes in a generated path are escaped so it builds on Windows (hazync#616). Never executed by Hazync.
- **`vendor/risc0-circuit-keccak-sys`** — 4.1.0. Modified in `build.rs` only (`HAZYNC_631_MSVC_FLAGS`): MSVC flags.
- **`vendor/risc0-build-kernel`** — 2.0.1. Modified in `src/lib.rs` only (`HAZYNC_631_MSVC_HOST_FLAGS`): host
  compiler flags for MSVC; the non-MSVC branch is unchanged.
- **`vendor/risc0-groth16-sys`** — 0.1.0. Modified in `kernels/cuda/groth16_coeffs.cuh` only
  (`HAZYNC_631_WIN_NO_MMAP`): the POSIX `mmap` path is gated off on MSVC, where SNARK wrapping fails with a clear
  error.
- **`vendor/sppark`** — 0.1.15, Apache-2.0, Copyright Supranational LLC
  (https://github.com/supranational/sppark). Modified in `sppark/ff/mont_t.cuh` only (`HAZYNC_631_QUALIFY_N`): a
  default argument qualified as `mont_t::n` so MSVC accepts it; the same value on every compiler.

## Our own code

- The Utreexo UTXO accumulator (`accumulator/`), the coinbase sparse Merkle tree (`coinbase-smt/`),
  libzync (`libzync/`), the host/prover driver (`prover/host/`), the guest glue
  (`prover/methods/guest/`), the libzync verifiers (`swiftsync-verify/`, `swiftsync-verify-ffi/`,
  `swiftsync-verify-wasm/`), the coordinator (`coordinator/`), and the retired CORE design's code under
  `history/` (including its range-state types and verifiers) are original work under the root MIT
  licence (the CORE `verifier` crate, `history/verifier/`, additionally declares Apache-2.0 in its
  `Cargo.toml`).

Nothing here overrides the terms of the components' own licences; consult each upstream project for
the authoritative text.

## Prior art

Not a dependency, and nothing of theirs is compiled in — recorded because the idea is theirs before
it was ours.

- **ZeroSync** — https://zerosync.org. Robin Linus and collaborators. The project that established
  proving Bitcoin chain state as a real engineering problem rather than a thought experiment.
  Hazync takes a different trade (Core's own consensus sources in a general-purpose zkVM, rather
  than a purpose-built circuit) and is downstream of theirs in the sense that matters: the question
  was already asked.
