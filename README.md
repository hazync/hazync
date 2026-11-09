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

Hazync doesn't rewrite them. Its core, **libzync**, compiles **Bitcoin Core's actual C++** — the real
`interpreter.cpp`, the real `SignatureHash`, the real `libsecp256k1` — to RISC-V and runs it inside a
zero-knowledge VM. Around that code it adds only what a proof needs: one batch check of each chunk's
signatures, an append-only **coin log** that makes a missing or double spend unprovable at every block, a
**counting trie** that answers questions by script (balance, history, "no such coin"), and **joins** that fold
independently proved blocks into one proof.

If Core accepts a block, so does this. Not because the behaviour was carefully matched, but because
it is the same code. [`docs/libzync/CONSENSUS_RULES.md`](docs/libzync/CONSENSUS_RULES.md) goes through
Core's rules one by one.

## The program

A proof names the exact program that produced it. The published libzync images are

```
mainnet  863c9ce35104740d8b6fa2972f0dd1b29289e3e67e5679400f0e8ecdbef69af5
signet   73b62530ca89eea376e7753f47ab4f16aed3be3138d6052d84d30510aed0f38d
```

and the build is reproducible: [`reproduce/libzync.Dockerfile`](reproduce/libzync.Dockerfile) gives
the same ids bit for bit ([`reproduce/LIBZYNC_IDS`](reproduce/LIBZYNC_IDS)), and CI rebuilds them on
every change. A proof made by any other program is a proof of something else — that is what stops
"trust us" from creeping back in.

## Where it actually stands

Honest, because overclaiming here would be the whole problem again. Every figure below was measured;
the sources are under [`experimental/libzync-bench/`](experimental/libzync-bench/).

**Works today**

- A real proof of mainnet from **genesis to block 200** by the published image: 3,031 s on one RTX PRO 6000,
  one 227 KB receipt that verifies in a browser
- A **coin check** page that verifies that proof on your own device and checks a server's answers
  about any script against it; a server that alters one word of an answer is refused
  ([`swiftsync-verify-wasm/zapp/`](swiftsync-verify-wasm/zapp/))
- On real mainnet state, blocks 200,000 to 300,000 on one card: **9.5x fewer proven cycles** and
  **7.0x less card time** than the original per-signature design
  ([`results/CYCLES_REAL_STATE.md`](experimental/libzync-bench/results/CYCLES_REAL_STATE.md))
- Blocks proved independently and **joined**: the joined proof equals the linear one exactly
- Reproducible builds, confirmed by an independent CI runner

**Not yet**

- **No independent audit.** The parts that are not Core's code (the coin log, the trie, the batch
  signature check, the aggregate) are where review matters most
- **No proof of the whole chain yet.** The state before every block is being prepared on our server;
  the chain will be proved from genesis under the published image, in parallel, and joined
- Following the chain's tip with libzync is built, not yet running live

## Help

The most valuable contribution is **trying to break it** — specifically, finding a case where
Hazync says a block is valid and Bitcoin Core would not. [`SECURITY.md`](SECURITY.md) says how to
report one.

| | |
|---|---|
| **What it proves and how** | [`docs/libzync/LIBZYNC.md`](docs/libzync/LIBZYNC.md) |
| **The proof statement, and what a verifier checks** | [`docs/libzync/SWIFTSYNC_PROOFS.md`](docs/libzync/SWIFTSYNC_PROOFS.md) |
| **What is built and what is open** | [`docs/libzync/ROADMAP.md`](docs/libzync/ROADMAP.md) |
| **Prove blocks on your GPU** | [`CONTRIBUTING.md`](CONTRIBUTING.md) |
| **Every document** | [`docs/README.md`](docs/README.md) |
| **Discussion** | [Delving Bitcoin](https://delvingbitcoin.org/t/running-cores-real-consensus-code-inside-a-zkvm/2811) |

Running it costs real money in GPU time. [What it costs and how to help pay for
it](https://hazync.org/donate/).

The original Hazync design, which proved each signature with Core's code directly (image `37987b85`), is
superseded by libzync; its documents are kept in [`docs/history/`](docs/history/README.md) and its code in
[`history/`](history/README.md).

## Licence

MIT (see [`LICENSE`](LICENSE)). The guest compiles in Bitcoin Core and libsecp256k1 (both MIT) with
the patches in `patches/`, none of which changes Core's consensus logic. `prover/` carries an
additional Apache-2.0 notice for the risc0-derived build scaffolding. Third-party components are
attributed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
