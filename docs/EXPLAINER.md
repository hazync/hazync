# Hazync, in plain English

> This page describes **Hazync v2** (the libzync engine), current since release v0.24.0. The version written for
> Hazync v1 is kept as a record in [`history/EXPLAINER.md`](history/EXPLAINER.md).

*No jargon. No maths background needed. If you have ever wondered "how does my Bitcoin wallet actually know the
money is real?", this is for you. It explains what Hazync is, why it matters, what is finished and what is not.*

---

## The problem: checking Bitcoin gets harder every year

Bitcoin has no bank in the middle. So how does anyone know a coin is real and has not been spent twice?

The answer is stubborn on purpose: **every full node checks everything itself, from the very beginning.** A new
Bitcoin node downloads the whole history, every transaction since 2009, and checks it: every rule, every coin, and,
in principle, every signature. Only then does it trust the balance sheet it ends up with.

That is what makes Bitcoin trustless. It is also the catch:

- It takes a capable computer many hours, often longer.
- The history only grows.
- Most people never do it. They use an app that *tells* them the balance and hope it is honest.

## The shortcut everyone already takes: "assume valid"

Here is something most Bitcoin users do not know. To make that first sync bearable, Bitcoin Core ships with a
setting called **assumevalid**. The developers write the name of one recent block into each release, and a new
node *skips checking the signatures* in every block before it. It still checks the other rules, but for the
signatures, by far the most expensive part, it trusts that the developers and the many people who reviewed the
release would have noticed a bad block.

That is a reasonable shortcut, and it has worked. But it is a shortcut built on trust in people, not on checking.

## The idea: a receipt you can check without redoing the work

Imagine a teacher with 10,000 exam papers to grade. Now imagine a special kind of stamp: the teacher grades
everything **once**, and produces a small sealed certificate that says *"I checked all 10,000 papers by these
exact rules."* Anyone can check the certificate in a moment and be sure the grading was done, without regrading a
single paper. If even one answer had been graded wrong, the certificate could not have been made.

That stamp is real. In computer science it is called a **zero-knowledge proof** (the name is unfortunate: here it
just means "a proof you can check quickly without re-running the work"). Hazync calls the certificate a **proof**
or **receipt**.

**Hazync applies that stamp to Bitcoin's history.** Someone does the full check once, every block, every
signature, and produces a small proof. Anyone else checks the proof in seconds and knows the work was done.

That is the difference from assumevalid: assumevalid says "trust that people would have noticed"; a Hazync proof
says "here is evidence that the checking was done, which you can verify yourself".

## What a proof says, exactly

A Hazync v2 proof says: **every block from Bitcoin's first block up to a named block follows Bitcoin's rules**,
and it carries a short summary of the state after that block (which coins exist, how much work the chain
contains, and a few other things other tools can build on).

Three honest limits come with that, and the checking tool tells you about each one:

- **It says the chain is valid, not that it is *the* Bitcoin chain.** Someone could, in principle, prove a valid
  but different chain. So the checker compares the proof's last block with a source you trust: your own node, a
  block explorer, or a block name you already have. If you skip that, it tells you it did not check.
- **It needs enough history to mean something.** A proof of only the first few thousand blocks is real, but cheap
  for an attacker to imitate, so the checker calls it a **demo** rather than **OK** until the proof reaches a block
  pinned into the program (block 970,000 on mainnet) and enough work.
- **It is only as good as the program that made it.** Each proof is tied to one exact program, named by a long
  fingerprint. Anyone can rebuild that program from the published source and check they get the same
  fingerprint ([`../reproduce/`](../reproduce/)).

The precise version, with every check, is [`SOUNDNESS.md`](SOUNDNESS.md).

## Why ours is built the way it is

To make the proof, you have to run Bitcoin's rules *inside* the proof machine. The tempting way is to **rewrite**
the rules in whatever language the machine likes. The trouble is that a rewrite is a photocopy, and Bitcoin's
rulebook is full of tiny, load-bearing quirks. A photocopy that differs in one comma proves the wrong thing.

**Hazync does not rewrite the hard part.** It takes Bitcoin Core's own source code (the part that checks scripts
and signatures, the transaction checks, the difficulty rules) and the same cryptography library Core uses, and runs
*that* inside the proof machine. There is no "did we translate it faithfully?" gap there, because there is no
translation.

We have to be exact about the rest, because "it's all Core's code" would not be true. Hazync v2 adds pieces of
its own:

- the **bookkeeping of coins**: a running log of every coin and whether it has been spent;
- a **searchable index** of coins by address, so questions like "does this address hold anything?" have provable
  answers;
- a way to **check all the signatures in a piece of a block in one batch**, which is much cheaper inside the proof
  machine than checking them one at a time;
- the **joining** of proofs: proving many blocks separately, on many machines, then stitching the proofs together.

These are ours, not Core's. That makes them exactly where we most want other people to look
([`EXTERNAL_REVIEW.md`](EXTERNAL_REVIEW.md)). Bitcoin Core's own test cases for scripts and transactions (22,666 of
them) have been run through a v2 build, and it agreed with Core on every one; details, including which build, are in
[`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5.

## What v2 changed

Hazync v1 worked, and proved a long stretch of history in public. Hazync v2 is a new engine for the same job. It
still runs Core's own code, but it is much cheaper to prove: on real Bitcoin blocks it needed up to about 20 times
less proving work than v1 (19.9 times on block 200,000; 9.5 times across eight blocks measured between 200,000 and
300,000). That matters because cost is what decides how fast the whole history gets proved.

v2 also makes the summary at the end of a proof useful to other tools: small separate programs, called **zAPPs**,
can take a Hazync proof and answer one question with it.

## What zAPPs are

A zAPP is a small program that starts from a Hazync proof and proves one specific answer, so you can check the
answer without trusting whoever gave it to you. Two exist today:

- **Header chain.** Every Bitcoin block has a short header, and the headers link together into a chain carrying all
  of Bitcoin's proof of work. Hazync has proved the whole header chain, from the first block to block 975,278, in
  one proof: about an hour on one graphics card, a 224 KB file that a web browser checks in a fraction of a second
  (39 ms, measured on a laptop). See [hazync.org/headers](https://hazync.org/headers/).
- **Coin check.** Is this coin still unspent, or was it spent, and where? The answer comes with a proof against the
  chain state. Today the demo runs against a proof of Bitcoin's first 200 blocks. See
  [hazync.org/zapp](https://hazync.org/zapp/).

More are planned. What a zAPP may rely on, and how to build one, is in [`libzync/ZYNCAPPS.md`](libzync/ZYNCAPPS.md).

## What works today, and what does not

**Working:**

- The v2 program, published as release v0.24.0, with its fingerprint reproducible from source.
- Real proofs from it: Bitcoin's first 200 blocks from the very beginning, checked by the published checker.
- The whole header chain in one proof, checkable in a browser.
- The public board at [hazync.org](https://hazync.org/), which switched to v2 on 12 November 2026 and restarted
  from the first block. Volunteers' graphics cards prove blocks one by one, and the proofs are joined together.
- Proving on Windows graphics cards as well as Linux.

**Not done yet:**

- **A proof of every block in Bitcoin's history.** The method works; it needs a lot of graphics-card time, and the
  board has only just started its v2 run. Until a proof reaches block 970,000, the checker will call board proofs a
  demo.
- **Keeping up with new blocks as they are mined.** Not running yet in v2.
- **A professional security audit.** Our own review rounds have found and fixed real bugs (listed in the open in
  [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5), but nobody has been paid to audit Hazync independently, and that
  is not the same thing. Nobody should rely on a Hazync proof for money until that has happened.

## Why this matters

If Hazync is finished:

- **Anyone can check Bitcoin's whole history quickly, on ordinary hardware,** by checking one proof instead of
  replaying fifteen years, and without the "trust the developers" shortcut of assumevalid.
- **Bitcoin gets easier to verify as it grows, not harder.**
- **It proves the existing rules were followed.** There is no new rulebook to trust: the rules in the proof are
  Core's own code.

## How you can help

- **Lend a graphics card.** The board splits the work into one job per block. [`../CONTRIBUTING.md`](../CONTRIBUTING.md)
  explains how to join, and [hazync.org](https://hazync.org/) shows your work landing.
- **Break it.** If you know cryptography or Bitcoin's internals, try to make the proof say "valid" for something
  Bitcoin would reject. [`EXTERNAL_REVIEW.md`](EXTERNAL_REVIEW.md) says where to start, and
  [`../SECURITY.md`](../SECURITY.md) how to report it privately.
- **Reproduce it.** Rebuild the program from source and check you get the published fingerprint.
- **Spread the word.** Send this page to one person who would care.

---

*The deeper versions: [`SOUNDNESS.md`](SOUNDNESS.md) is the exact security claim, [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md)
the technical overview of the v2 engine, [`../SECURITY.md`](../SECURITY.md) how to report a flaw, and
[`../CONTRIBUTING.md`](../CONTRIBUTING.md) how to join the proving run. We keep the caveats in writing on purpose:
a project like this is only worth anything if its claims are true.*
