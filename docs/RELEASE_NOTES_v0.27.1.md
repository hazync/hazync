**A fix release: the Linux GPU prover works again, and every release is now proved on real GPUs before it ships.** The images are unchanged (mainnet `46666b6f`, signet `01a80f17`, headers `de54b94f`); nothing is re-proved, and the board keeps its chain.

## The Linux CUDA prover (fixed)

The Linux CUDA host in **v0.26.1 and v0.27.0** (`hazync-host-x86_64-linux-gnu-cuda`) stops on its first GPU step: a segfault, or `cudaStreamSynchronize … "context is destroyed"`. v0.26.0's host proved the same block on the same card.

- **Cause:** a stale build cache, not the source. #739 changed a vendored sppark header and source file, and sppark's build script, as vendored, did not ask to be re-run when its own files changed. So the release build reused an old `libsppark_cuda.a` while other crates were rebuilt against the new header, giving two layouts of one structure in one binary.
- **The Windows prover was not affected.** CI builds it fresh every time.
- **v0.27.1's host is built clean** from the same source. A clean build of it, on one RTX PRO 6000 (Blackwell), proved mainnet blocks 170, 1,000, 100,000, 200,000, 270,000, 280,000, 290,000, 300,000 and 400,000 from real chain state, and every receipt verified. Block 976,000 took 808 s.
- **If you prove on Linux with a GPU,** update to v0.27.1. Until then, v0.26.0's Linux CUDA host makes the same image and its proofs are accepted.

## Every release is now proved on real GPUs first

`scripts/release.sh` refuses to publish unless the exact release binaries have proved real blocks on real cards (#781):

- **Linux:** a gate (`scripts/release_gpu_gate.py`) rents one pre-Blackwell and one Blackwell card, proves block 170 and block 200,000 with the staged host, and verifies the receipts. Pods are always released, and spend is capped.
- **Windows:** the GPU host is proved on a real card (`scripts/release-gpu-gate-windows.ps1`) and recorded against the staged zip.
- **Clean builds:** release builds are clean, and the vendored GPU crates now rebuild when a shared header changes, so a warm cache can no longer mix layouts.

## The Linux verifier runs on Ubuntu 22.04 again

`hazync-verify-x86_64-linux-gnu` (and the aarch64 build) needed glibc 2.39, so v0.26.1's and v0.27.0's did not start on Ubuntu 22.04 or Debian 12 (v0.26.0's x86_64 verifier started but warned on every run). Every Linux binary is now built in the same `ubuntu:22.04` image as the hosts (glibc 2.34), and the release refuses any Linux binary that needs more.

## Also since v0.27.0

- **Header-chain follower:** its script now expects the MMR image's guest (`de54b94f`); the released v0.27.0 script still expected the old one (#773).
- **Measured cost curve** (`experimental/libzync-bench/cost-curve/`):
  - proving cycles for 25 heights across the chain's history;
  - real GPU times for 11 blocks;
  - a measured rate of about 24 s per block plus about 1 s per million cycles on an RTX PRO 6000;
  - a whole-chain projection of about 332 T cycles, about 116,000 card-hours (#782).
- **Cycle profiles** of block 962,000 from the v0.22.1 and v0.27.0 release binaries, reproducible with one script (#778).
- **Docs:**
  - the README's "How you can help", the Windows installer first, and every current-state document brought up to date (#776);
  - Blackwell guidance and one worker per card in the Windows app (#780);
  - prep's memory guide (#775).
- **Corrections:**
  - the v0.27.0 notes said the hints bit-order question had been asked; it had not;
  - SwiftSync is now credited as the sources state it: originally proposed by Ruben Somsen.
- **CI:** the Bitcoin Core download in the rig job retries (#777).

## What changes for you

- **Linux GPU provers:** update to v0.27.1.
- **Linux verifier users on 22.04 or Debian 12:** update to v0.27.1.
- **Windows app users, CPU provers, board operators, website:** nothing; the images are unchanged.

