# What still needs outside eyes

> This page describes **Hazync v2** (the libzync engine), the images published in release v0.24.0. The guide
> written for Hazync v1 (the CORE guest, its accumulator, field backend and ghostd adoption) is kept in
> [`history/EXTERNAL_REVIEW.md`](history/EXTERNAL_REVIEW.md).

Everything in Hazync v2 has been reviewed. None of it has been reviewed by someone who could be wrong in a
different direction than we are. No test we write closes that gap, so this page says where an hour of an outsider's
time is worth most.

## What we already have, stated precisely

**Cold review rounds, not an audit.** Fresh reviewers were given the code, not the author's conclusions: rounds 1-8
on the earlier design, then seven more (E2 r1-r7) on the current one. Every round, its findings and its fix are in
[`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5. They found real soundness bugs, among them a flag that switched coin
accounting off, a base-step value that skipped verifying the previous proof (a forgery), witness bytes unbound
across chunks and a signed BIP68 compare. **No commissioned professional audit has happened**
([`../SECURITY.md`](../SECURITY.md)).

**Tests that can fail**, each naming the image it ran on ([`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §5): a rig of
crafted blocks run through the guest and real Bitcoin Core 28.0 on regtest (96/96 agree on the frozen core); Core's
`script_tests`, `tx_valid` and `tx_invalid` with every flag variant (22,666 / 22,666 agree, on build c10e12); forged
signature hints (0 Core-invalid cases pass, c10e11); joins of 200 leaves byte-identical to the linear chain; independent
Python references for the coin log and the block forest.

The rule-by-rule comparison with Core is [`libzync/CONSENSUS_RULES.md`](libzync/CONSENSUS_RULES.md): 59 SAME,
3 STRICTER, 0 WEAKER, 0 NOT ENFORCED. The security claim is [`SOUNDNESS.md`](SOUNDNESS.md).

## Before reading anything, check something

A few minutes on any Linux x86-64 box: no GPU, no chain data. Build the verifier and run its test vectors, which
include a real GPU proof of mainnet blocks 1..200 under the published image:

```sh
cargo build --release --manifest-path swiftsync-verify/Cargo.toml
python3 swiftsync-verify/vectors/run_vectors.py swiftsync-verify/target/release/swiftsync-verify   # 13/13
swiftsync-verify/target/release/swiftsync-verify --network mainnet --product --mempool \
    swiftsync-verify/vectors/mainnet_1_200_????????.rcpt
```

The vector is a real proof of mainnet 1..200 by v0.24.0's image `8888d59c…`, which v0.26.0's `1ff5a129…` continues at
block 2000: the verifier sees the receipt names that earlier built-in image and checks it as that image (first line of
its output).

The last command should answer **DEMO** (exit 2): valid from genesis, below Core's minimum chainwork and the
checkpoint. `--mempool` makes one request to mempool.space to compare the tip hash with block 200; or pass
`--expect 200:<hash>` from your own node. The release binary `hazync-verify-x86_64-linux-gnu` is the same verifier.

The board's current genesis-anchored proof is served at `https://api.hazync.org/api/spine/proof`; it will also be
DEMO until its chain passes block 970,000.

## The list, in the order worth spending on

### 1. Reproduce the image id: highest value, lowest cost

Every claim is about the program the image id names. Nobody outside this project has re-derived it.

```sh
docker build -t hazync-libzync-repro -f reproduce/libzync.Dockerfile .
docker run --rm hazync-libzync-repro      # prints the ids and ELF hashes
```

Compare with [`../reproduce/LIBZYNC_IDS`](../reproduce/LIBZYNC_IDS) (mainnet `1ff5a129…`, which continues `8888d59c…` at block 2000; signet
`28e39600…`; headers zyncAPP `2db34e12…`) and with the ids built into the verifier (`IMAGES` in `swiftsync-verify/src/lib.rs`; a
unit test checks the two agree). The id depends on absolute build paths, so a laptop build differs by design: only
the container's output counts. Also worth a look: what the container pins (base image digest, `provision-vps.sh`,
`scripts/libzync-base.sh`, the patches) and what it does not.

### 2. The batch signature check

Not Core's code, on the path of every signature. libsecp parses and range-checks every key and signature; for a
hinted signature, the group equation R = u1·G + u2·Q is deferred to one Fiat-Shamir-weighted multi-scalar
multiplication per chunk.

- `prover/methods/guest/src/batch_ecdsa.rs`, `prover/methods/guest/src/bigint2_ecmult.rs`
- the libsecp hook it plugs into: `patches/libzync/0102-secp-libzync-batch-hooks.patch`
- the glue: `prover/methods/guest/verify_input.cpp`

The claim to test: a lying host can only make a valid signature fail, never make an invalid one pass. Open, and
named as such: the x = r + n case is infeasible for real r and effectively untested
([`libzync/ROADMAP.md`](libzync/ROADMAP.md) I1).

### 3. The coin log

Hazync's replacement for Core's coin database: an append-only forest of perfect Merkle trees in birth order, where a
spend flips the coin's leaf in place. A bug here reaches backwards: every proof built on a broken commitment is
worthless, and the repair is re-proving from genesis.

- `libzync/src/coinlog.rs`, `libzync/src/coinlog_forest.rs` (compiled into the guest by path)
- spec: [`libzync/COINLOG.md`](libzync/COINLOG.md); leaf bytes: [`libzync/UTREEXO_FORMAT.md`](libzync/UTREEXO_FORMAT.md) §2
- independent reference: `scripts/libzync/coinlog_ref.py`

The claim to test: a missing or already-spent coin cannot be spent, at every block.

### 4. The counting trie

Keyed by (SHA-256(scriptPubKey), outpoint), with history and per-script counts and values. It answers absence,
history and balance, which zyncAPPs rely on. It is updated in its own receipt (guest mode 17) and verified inside the
aggregate; its old root must come from the previous `ChainState`, never from the witness.

- `libzync/src/ttree.rs`, `libzync/src/trie_fact.rs`, `libzync/src/trie_model.rs`, `libzync/src/trie_store.rs` (host store)
- `trie_update()` in `prover/methods/guest/src/main.rs`
- contents, including the BIP30 edge cases: [`libzync/ZYNCAPPS.md`](libzync/ZYNCAPPS.md) §2

### 5. The block rules applied from the chunks' facts

The aggregate never re-parses a transaction: it checks the chunk-committed facts against the merkle-committed bytes
and enforces fees, weight, sigops, BIP30/34/68 and that every input is covered exactly once. This is Hazync Rust
transcribing `ConnectBlock` and `CheckBlock` rule by rule, and the BIP68 bug the reviews found was here.

- `prover/methods/guest/src/agg_lite.rs`, `aggregate()` in `prover/methods/guest/src/main.rs`, `libzync/src/facts.rs`
- the Core functions it lifts: `prover/methods/guest/core_block_rules.cpp`, `core_tx_rules.cpp`
- the row-by-row map: [`libzync/CONSENSUS_RULES.md`](libzync/CONSENSUS_RULES.md)

### 6. Handover and joins

A join (guest mode 18) verifies two adjacent proofs of the same image and requires the right one to start where the
left one ends; the board builds its whole chain this way. A handover lets a new image continue a chain from a pinned
predecessor. The published images pin no handovers (`prover/methods/guest/src/lineage/`), but the code is in them.

- `join()` and `aggregate()` in `prover/methods/guest/src/main.rs`; `prover/methods/guest/src/handover_pin.rs`
- spec: [`libzync/HANDOVER.md`](libzync/HANDOVER.md); block forest reference: `scripts/libzync/block_forest_ref.py`

The claim to test: an untrusted starting state, a gap, a swap, a repeat or another image's proof can only fail a
join, never produce a proof of a chain that was not proved.

### 7. The prep pass that builds leaf bundles

Every block on the board is proved from a leaf bundle (`leaf_<h>.bin`, magic `LZLEAF02`) holding the state before the
block and every witness its proof needs. Prep builds them natively, outside the zkVM.

- `prover/host/src/prep.rs`, `prover/host/src/lz.rs`; on the board box, `coordinator/deploy/libzync/libzync-prep-run.sh`
- how the coordinator uses a bundle's `prev` to check a range's claimed start: `prover/host/src/lz_coord.rs`

Prep is meant to be outside the trust base: a wrong state should cost liveness only. That is the claim to test, in
particular that a range proved from a wrong `prev` can never join onto the genesis-anchored chain.

### 8. The verifier, its WebAssembly and C builds, and the test vectors

A bug here accepts a proof nobody should.

- `swiftsync-verify/src/lib.rs` (`ChainState`, `verify`, `genesis_anchor`), `main.rs`, `headers.rs` (the header-chain
  zyncAPP), `zync_path.rs`
- the browser build `swiftsync-verify-wasm/` (shipped as `hazync-verify.wasm`) and the C library
  `swiftsync-verify-ffi/`
- `swiftsync-verify/vectors/` (`manifest.json`: 13 cases, plus reference values a second implementation should
  reproduce)

Check each step of [`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) §5 against the code, and that the three
builds give the same answers. The browser verifier's bytes are reproducible only with the same Rust toolchain as the
release (a known follow-up).

### 9. The coordinator under a hostile-coordinator model

The coordinator is not in the trust base for validity: it verifies receipts and hands out work. But a worker
trusts the coordinator it is pointed at for what it downloads, and anyone can run one
([`RUN_YOUR_OWN_COORDINATOR.md`](RUN_YOUR_OWN_COORDINATOR.md)). Worth a look: `coordinator/server.py` (what it accepts
and how it chains ranges by link digest) and the worker `coordinator/hazync`, whose `get()` still reads a whole
response with an unbounded `.read()`; anything that treats the coordinator's answer as well-formed needs the same look.

### 10. Node adoption

No node adopts a Hazync v2 proof yet: adoption today is through the verifiers ([`libzync/LIBZYNC.md`](libzync/LIBZYNC.md)
§7). The integration surface a node would use is the C library, `swiftsync-verify-ffi/` (`include/swiftsync_verify.h`),
which returns the proven height, tip, work and coin count so the caller can compare the tip with its own chain. Review
it as an API: can a caller misread an answer, or skip the chain-identity check without noticing? The ghostd adoption
reviewed under v1 adopts v1 proofs; see the v1 guide.

## Not on this list, deliberately

**Core's consensus code itself.** It is compiled from a pinned tag with three reviewed patches; auditing it here
would be auditing Bitcoin Core. The pinning is what needs checking (item 1), and the patches (items 1 and 2).

## How to report what you find

- **A way to make an invalid block prove valid, or any other security issue:** report it **privately** through
  GitHub's private vulnerability reporting, <https://github.com/hazync/hazync/security/advisories/new>, as
  [`../SECURITY.md`](../SECURITY.md) describes. Please do not open a public issue for it.
- **Anything else** (a wrong doc, a test that cannot fail, a build that does not reproduce): a GitHub issue on
  <https://github.com/hazync/hazync>.

The binaries you review should be the released ones. Each release's `SHA256SUMS.txt` is PGP-signed by the
maintainer key, fingerprint:

```
777F E81F 8CC0 77FD 3D08  055E 852C 2B31 90F5 B928
```

Fetch the key from a second source, `https://github.com/defenwycke.gpg`, as [`../SECURITY.md`](../SECURITY.md)
explains: a fingerprint published in the repository you are auditing is not independent evidence.
