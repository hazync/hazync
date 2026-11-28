# Third-party notices

Hazync is MIT-licensed (see [`LICENSE`](LICENSE)). It builds on, and in some cases compiles in,
third-party components under their own licences. This file records them.

## Compiled into the zkVM guest

The patch sets below are the ones Hazync v2's published images (v0.24.0) are built with, as applied by
`scripts/libzync-base.sh`; Hazync v1's CORE build applied a subset. The engine guest (OpenVM, engine/guest/base.sh) uses
the same Core base and adds `engine/guest/core-0201-sha-openvm-native.patch` and `core-0202-sha-finalize-in-place.patch`
to Bitcoin Core, and `secp-0201-openvm-batch-liftx.patch` and `secp-0202-sha-openvm-native.patch` to a pristine
libsecp256k1 v0.5.1, without the RISC Zero field and group patches (0005 0006 0008 0012 0013).

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
  `risc0-build` `=3.0.7`, and the rzup cross-toolchain — Apache-2.0, Copyright (c) RISC Zero, Inc. The prover and
  every release up to v0.27.1, the last on RISC Zero; since CUTOVER C8 (28.11.2026) this tree neither builds nor links
  any RISC Zero crate (the engine below proves). The `prover/` crate was scaffolded from the risc0 project template
  and carries an additional Apache-2.0 notice at [`prover/LICENSE`](prover/LICENSE) covering that risc0-derived build
  scaffolding; RISC Zero's serde lives on in `engine/journal` (below).
- **RustCrypto `sha2`** — MIT/Apache-2.0, pinned to an immutable commit, routed through the risc0
  SHA-256 accelerator (byte-identical output).

- **OpenVM** v2.1.0 (https://github.com/openvm-org/openvm) and **stark-backend** v2.1.0
  (https://github.com/openvm-org/stark-backend) — MIT OR Apache-2.0. The engine prover and every verifier on branch
  `engine/openvm` (engine/ENGINE.md), pinned by tag. Patched at build time, never vendored: `engine/wasm-verify/
  openvm-abi-stable.patch`, `engine/handover/openvm-verify-nozstd.patch` and `engine/handover/stark-sdk-nologging.patch`
  (the WASM and handover verifier builds; engine/scripts/openvm_src.sh applies them to the pinned commits).
- **`engine/journal`** (crate `hazync-journal`) and **`engine/platform/risc0-zkvm`** — the journal word encoding
  (`src/serde`) is risc0-zkvm 3.1.0's `src/serde`, copied with one change (`WORD_SIZE` a local constant), Apache-2.0,
  Copyright (c) RISC Zero, Inc.; the files keep RISC Zero's licence header. The engine guests and every verifier use it.

## Vendored and modified (until CUTOVER C8)

Until C8 step 4f (28.11.2026) `vendor/` held six RISC Zero crates (risc0-zkvm 3.1.0, risc0-circuit-rv32im-sys,
risc0-circuit-keccak, risc0-circuit-keccak-sys 4.1.0, risc0-build-kernel 2.0.1, risc0-groth16-sys 0.1.0; Copyright (c)
RISC Zero, Inc.) and sppark 0.1.15 (Copyright Supranational LLC), Apache-2.0, each with prover-side changes marked in
its source (segment distribution, a balanced join tree, the Windows / MSVC CUDA build). They left with the RISC Zero
backend; v0.27.1's source carries them and this file's text as it was.

## Our own code

- The Utreexo UTXO accumulator (`accumulator/`), the coinbase sparse Merkle tree (`coinbase-smt/`),
  libzync (`libzync/`), the host/prover driver (`prover/host/`), the guest glue
  (`prover/methods/guest/`), the libzync verifiers (`swiftsync-verify/`, `swiftsync-verify-ffi/`,
  `swiftsync-verify-wasm/`), the coordinator (`coordinator/`), and the retired CORE design's code under
  `history/` (including its range-state types) are original work under the root MIT licence (the CORE
  `verifier` crate, deleted from `history/` by CUTOVER C8 step 3, additionally declared Apache-2.0 in its `Cargo.toml`).

Nothing here overrides the terms of the components' own licences; consult each upstream project for
the authoritative text.

## Fonts (the Ooga Booga Land page)

- **Schibsted Grotesk** and **Spline Sans Mono** — SIL Open Font License 1.1, not MIT. Copied as `.woff2` into
  `ooga-land/public/fonts/` so the page loads nothing from another origin; the licence text and the copyright
  lines travel with them in `ooga-land/public/fonts/OFL.txt`.

## Prior art

Not a dependency, and nothing of theirs is compiled in — recorded because the idea is theirs before
it was ours.

- **ZeroSync** — https://zerosync.org. Robin Linus and collaborators. The project that established
  proving Bitcoin chain state as a real engineering problem rather than a thought experiment.
  Hazync takes a different trade (Core's own consensus sources in a general-purpose zkVM, rather
  than a purpose-built circuit) and is downstream of theirs in the sense that matters: the question
  was already asked.
