# Hazync

**Sync Bitcoin from a proof, not from trust.**

---

## The problem

To use Bitcoin without trusting anyone, your node downloads the entire chain and re-checks every
signature ever made — hundreds of gigabytes and hours of work, and it grows every day. Most people
don't. They use someone else's node and take its word for it.

That cost is the reason Bitcoin gets less decentralised over time.

## The idea

Check the work **once**, produce a proof that it was done correctly, and let everyone else verify
that proof in milliseconds.

This isn't new. What's hard is the honest version of it.

## Why this one is different

Every other attempt rewrites Bitcoin's consensus rules in a language a prover can handle. That
rewrite then has to match Bitcoin Core **exactly, in every edge case, forever** — including the bugs
Core can never fix because they're now consensus. Nobody can prove that about a rewrite.

Hazync doesn't rewrite them. **Hazync v2** (the libzync engine, current since release v0.24.0) compiles **Bitcoin Core's actual C++** — the real
`interpreter.cpp`, the real `SignatureHash`, the real `libsecp256k1` — to RISC-V and runs it inside a
zero-knowledge VM. Around that code it adds only what a proof needs: one batch check of each chunk's
signatures, an append-only **coin log** that makes a missing or double spend unprovable at every block, a
**counting trie** that answers questions by script (balance, history, "no such coin"), and **joins** that fold
independently proved blocks into one proof.

If Core accepts a block, so does this. Not because the behaviour was carefully matched, but because
it is the same code. [`docs/libzync/CONSENSUS_RULES.md`](docs/libzync/CONSENSUS_RULES.md) goes through
Core's rules one by one.

## The program

A proof names the exact program that produced it. The published Hazync v2 images, built on RISC Zero 3.1.0
(release [v0.26.0](https://github.com/hazync/hazync/releases/tag/v0.26.0)), are

```
mainnet  1ff5a129d1ebc2022262f1249c1b41a0c096b8d5d16f985d7fc018138c67c66a
signet   28e396006e97a8075daeb9fb12ce3ef1b3a20e44eac935256a1c2e8c3d8d57f3
headers  2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632   (the header-chain zyncAPP)
```

The mainnet image continues v0.24.0's `8888d59c…` at block 2000 by a handover ([`docs/HANDOVER.md`](docs/HANDOVER.md)):
the board's blocks below 2000 are that image's proofs, and the verifier keeps it.

and the build is reproducible: [`reproduce/libzync.Dockerfile`](reproduce/libzync.Dockerfile) gives
the same ids bit for bit ([`reproduce/LIBZYNC_IDS`](reproduce/LIBZYNC_IDS)), and CI rebuilds them on
every change. A proof made by any other program is a proof of something else — that is what stops
"trust us" from creeping back in.

## Where it actually stands

Honest, because overclaiming here would be the whole problem again. Every figure below was measured;
the sources are under [`experimental/libzync-bench/`](experimental/libzync-bench/)
([`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) §4 lists every figure and where it comes from).

**Works today**

- **The public board proves Hazync v2.** [hazync.org](https://hazync.org/) switched to it on 12 November 2026 and
  restarted from genesis under `8888d59c` (continued from block 2000 by `1ff5a129`, a handover that keeps every
  earlier proof, [`docs/HANDOVER.md`](docs/HANDOVER.md)): anyone with an NVIDIA card claims a block, proves it and submits it,
  and proved blocks are joined into one genesis-anchored proof ([`CONTRIBUTING.md`](CONTRIBUTING.md)). Its first
  proving run, on 4× RTX PRO 6000, proved 1,287 blocks in 2 h 32 min of proving (about 510 an hour), $27.13 in all,
  about $0.021 a block on these early, nearly empty blocks
  ([`RESULT_board_run_2026-11-12.txt`](experimental/libzync-bench/RESULT_board_run_2026-11-12.txt))
- **The whole header chain in one proof:** genesis to block 975,278 under the headers zyncAPP `2db34e12`, 59.4 min on
  one RTX PRO 6000 (about $2.50), a 224 KB receipt that the WebAssembly verifier checks in 39 ms (on a laptop,
  under Node). A follower on the board's server keeps it at the tip
- A real proof of mainnet from **genesis to block 200** by the published image: 3,074 s on one RTX PRO 6000 (RISC
  Zero 3.1.0), one 227 KB receipt that verifies in a browser
- **zyncAPPs**, separate programs that answer one question from a core proof ([`docs/libzync/ZYNCAPPS.md`](docs/libzync/ZYNCAPPS.md)),
  and a **coin check** page that verifies that proof on your own device and checks a server's answers
  about any script against it; a server that alters one word of an answer is refused
  ([`swiftsync-verify-wasm/zapp/`](swiftsync-verify-wasm/zapp/))
- On real mainnet state, eight blocks from 200,000 to 300,000 on one card: **9.5x fewer proven cycles** and
  **7.0x less card time** than Hazync v1, the original per-signature design; up to 19.9x fewer cycles and 15.1x
  less card time on block 200,000
  ([`results/CYCLES_REAL_STATE.md`](experimental/libzync-bench/results/CYCLES_REAL_STATE.md))
- Proving on **Linux and on native Windows** with an NVIDIA card: the Windows CUDA host
  (`hazync-host-x86_64-windows-cuda.zip`) proved mainnet blocks 170 and 400,000 on an RTX PRO 6000, verified again on
  Linux ([`experimental/libzync-bench/windows/`](experimental/libzync-bench/windows/), measured on the v0.23.0 build)
- Blocks proved independently and **joined**: the joined proof equals the linear one exactly
- Reproducible builds, confirmed by an independent CI runner

**Not yet**

- **No independent audit.** The parts that are not Core's code (the coin log, the trie, the batch
  signature check, the aggregate) are where review matters most
- **No proof of the whole chain's blocks yet** (the header chain is proved, above). The board is proving them
  from genesis; a prep pass on our server writes the state before each block, a little ahead of the provers
- Following the chain's tip with Hazync v2 is not running yet

## Help

The most valuable contribution is **trying to break it** — specifically, finding a case where
Hazync says a block is valid and Bitcoin Core would not. [`SECURITY.md`](SECURITY.md) says how to
report one.

| | |
|---|---|
| **In plain words** | [`docs/EXPLAINER.md`](docs/EXPLAINER.md) |
| **What it proves and how** | [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) |
| **The proof statement, and what a verifier checks** | [`docs/libzync/SWIFTSYNC_PROOFS.md`](docs/libzync/SWIFTSYNC_PROOFS.md) |
| **What is built and what is open** | [`docs/libzync/ROADMAP.md`](docs/libzync/ROADMAP.md) |
| **Prove blocks on your GPU** | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| **The current release** | [`docs/RELEASE_NOTES_v0.24.0.md`](docs/RELEASE_NOTES_v0.24.0.md), and every release in [`CHANGELOG.md`](CHANGELOG.md) |
| **Every document** | [`docs/README.md`](docs/README.md) |
| **Discussion** | [Delving Bitcoin](https://delvingbitcoin.org/t/running-cores-real-consensus-code-inside-a-zkvm/2811) |

Running it costs real money in GPU time. [What it costs and how to help pay for
it](https://hazync.org/donate/).

**Hazync v1**, the original design, which proved each signature with Core's code directly (image `37987b85`), is
superseded by Hazync v2. Its last release is [v0.22.1](https://github.com/hazync/hazync/releases/tag/v0.22.1), which
still verifies v1 proofs; its board reached frontier 144,769 and is kept read-only as the record. Its documents are
in [`docs/history/`](docs/history/README.md) and its code in [`history/`](history/README.md). Release v0.23.0, the
first with libzync, is **withdrawn**: it was built on RISC Zero 3.0.5, whose soundness bugs 3.1.0 fixes
([`CHANGELOG.md`](CHANGELOG.md)).

## Licence

MIT (see [`LICENSE`](LICENSE)). The guest compiles in Bitcoin Core and libsecp256k1 (both MIT) with
the patches in `patches/`, none of which changes Core's consensus logic. `prover/` carries an
additional Apache-2.0 notice for the risc0-derived build scaffolding. Third-party components are
attributed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
