# `swiftsync-verify`: check a proof that a SwiftSync hints file is correct

A 1.8 MB standalone verifier for libzync proofs (`docs/libzync/SWIFTSYNC_PROOFS.md`). No prover, no Bitcoin Core
build, no GPU, no chain data. It runs every check of the proposal's §5, and step 7 too if you point it at your node.

```sh
cargo build --release --manifest-path swiftsync-verify/Cargo.toml
swiftsync-verify/target/release/swiftsync-verify --network signet proof.rcpt signet.hints
swiftsync-verify/target/release/swiftsync-verify --network signet --bitcoin-cli "bitcoin-cli -signet" proof.rcpt signet.hints
```

| verdict | exit | meaning |
|---|---|---|
| `SWIFTSYNC_VERIFY_OK` | 0 | a genesis-anchored proof: the file is exactly right for the chain printed, at its stop height |
| `SWIFTSYNC_VERIFY_DEMO` | 2 | every check passed against a state given with `--anchor`: **never** a proof of the file |
| `SWIFTSYNC_VERIFY_FAIL` | 1 | anything else, including unreadable input |

**What it checks** (§5 of the proposal):
1. the receipt is a real proof (no dev-mode `FakeReceipt`, `RISC0_DEV_MODE` unset) of image M;
2. the journal is that image's `ChainState`;
3. `anchor_id` is the network's genesis state for M (or the `--anchor` state, giving at most DEMO);
4. to 6. the proof's height is the file's stop height, the ECMH total is the identity, and `ss_hints` is the
   file's hint hash;
7. with `--bitcoin-cli "<command>"`, your node's block at that height is the proven tip, with the same
   chainwork; otherwise the tip and work are printed and the verdict line says step 7 was not run;
8. to 11. the profile the image commits (scripts checked or not, network), canonical encoding of the file, and
   `time-too-new` on this machine's clock.

**Proof bundles.** `swiftsync-verify [--network ...] bundle <receipt> <file.hints> <out.sspf>` writes one file
holding the receipt and a header naming network, image, height, tip, work, `ss_hints` and the hints file's
SHA-256 (`docs/libzync/PROOF_BUNDLE.md`). A bundle is accepted wherever a receipt is; its header is checked against
the verified result and never trusted.

**Test vectors** for this or any other verifier: `vectors/` (14 cases with expected exit codes, plus reference
values; `python3 vectors/run_vectors.py <verifier>`).

**The image id is the trust root.** The built-in ids (`src/lib.rs`, `IMAGES`) are the current round-8 images. A
reader should rebuild M from the published commit and pass it with `--image <hex>` rather than trust this file
(a reproducible build kit is planned). `--image` is printed on the first line of the output.

**Tests:** `cargo test --release` (genesis work by long division, the empty-SMT root and the genesis `anchor_id`
vectors of §5 step 3), and `bash swiftsync-verify/test.sh` from the repo root (15 end-to-end cases on the real
round-4 signet receipt and corrupted inputs; output in `test.out`). That receipt starts from a supplied state at
2,500, so the best it can reach is DEMO; no genesis-anchored receipt exists yet.

**Shares code with the prover** only for the hint hash (`libzync::ss_hints::step`) and the hints-file decoder
(`hintsfile` 0.2.1). `scripts/libzync/ss_hints_ref.py` is an independent Python implementation of the hints side
from the spec text alone, to check both.
