# The deposit-proof zyncAPP

Status (15.11.2026): built on branch `feat/zapp-deposit`, tested on real receipts, not released. No new image id: it
checks two proofs that already exist. Code: `swiftsync-verify/src/deposit.rs` (verifier), `swiftsync-verify deposit`
(CLI), `verify_deposit` / `verifyDeposit` (WASM), `zync-deposit` (bundle builder). Tests: `swiftsync-verify/src/deposit/tests.rs`,
`experimental/libzync-bench/deposit/test_deposit.sh` and `test_deposit_wasm.mjs`.

## 1. The statement

One file (a *deposit bundle*) that a bridge, rollup or other chain's operator checks, with its own question, to learn:

> transaction T (and, if asked, its output v holding this amount and scriptPubKey, or an OP_RETURN output carrying
> this payload) is at position i of block h; there are N blocks on top of it as of tip X (height, hash, chainwork W);
> every block from genesis to the core tip Hc was fully validated (scripts and signatures, by Bitcoin Core's consensus
> code compiled into the guest); blocks Hc+1..X are checked for proof of work and Core's header rules only.

The caller supplies T, v, the amount, the script or payload, the depths, the network and the image ids. The bundle
carries no txid and no image id. The raw transaction must hash to the caller's T (the header zAPP review's rule:
an answer that is a valid proof for another transaction is refused). A statement is returned only when every check
passed. Verdicts as elsewhere: `OK` (0), `DEMO` (2: the tip proof is below Core's `nMinimumChainWork`), `FAIL` (1).

## 2. How the transaction is bound to a fully validated chain

Two proofs exist:

- the **core proof** (`46666b6f…`, v0.26.0; its proofs from the earlier `8888d59c…` still verify by id): every block
  0..Hc fully validated. Its `ChainState` holds a block forest with one leaf per block: the block hash, the coin
  state after it and (from v0.26.0) its undo hash. It reaches Hc, which today is well below the tip (the board is
  re-proving from genesis).
- the **header-chain proof** (`de54b94f…`): proof of work and Core's header rules 0..Ht, with an MMR of every block
  hash. It follows the tip.

Two ways to join them were considered.

**(a) Composition in the verifier (chosen).** The verifier checks both receipts, then:

1. *binding*: the core tip hash is the header MMR's leaf at height Hc (one MMR path). Both are hash chains from
   genesis, so the header chain's blocks 0..Hc are exactly the core's (a difference would be a SHA-256d collision);
2. *the block*: block h (h <= Hc) is a leaf of the core block forest (header, coin state, undo hash, path), so it is
   one of the fully validated blocks;
3. *the transaction*: it is at position i of that block by its merkle branch, with the tree depth pinned by the
   block's coinbase and its branch; a 64-byte transaction is refused (it cannot be told apart from an inner merkle
   node; CVE-2012-2459's family; BIP 54 would forbid it). The outputs are parsed from the bytes that hash to T;
4. *depth*: N = Ht - h + 1 (Hc - h + 1 without a header proof), of which Hc - h + 1 are fully validated.

A block above Hc can be shown only by the header MMR: proof of work only. It is refused unless the caller passes
`--allow-pow-only-block`, and the statement then says so.

**(b) A composed receipt.** A small guest verifies the core and header receipts as assumptions (`env::verify`),
makes the same four checks and commits one statement. Built to measure it
(`experimental/libzync-bench/deposit/tools/`, not a published image; raw output in `RESULT_measured.txt`): **816,427 cycles, one segment**, journal 1,316
bytes, on the real bundle (core 0..200, header chain 0..975,278, block 170). Proved on hazync-coord's CPU through r0vm
3.1.0 in 293.4 s (a 225,858-byte succinct receipt), wrapped in Groth16 in a further 88.5 s (3,129 bytes). A
tampered branch makes the guest panic. Its image id (`184e81af…`, laptop build) is not reproducible as built: a
published version would go through `reproduce/` like the others.

**Why (a).** It adds no image id to review, reproduce and pin (the trust is the two images already published), no
proving per deposit or per tip, and the verifier is a pure function that already runs in the CLI and in WASM (a
browser). (b) is only needed when ONE receipt must carry the statement, that is when a contract checks it on chain
(§6). (b) reads the same bundle, so it can be added later without changing anything a bridge already uses.

## 3. What full validation adds, and what it does not

Compared with a header-only (SPV) light client, which knows that a header chain has work and that a transaction's
merkle branch reaches one of its headers:

| question | header-only client | this bundle |
|---|---|---|
| the block's transactions are valid (scripts, signatures, amounts, no double spend, subsidy) | not checked: an invalid block on a chain with work passes | checked for every block 0..Hc |
| the inputs the deposit spends existed and were unspent | not checked | checked (the block was validated against the coin set) |
| every earlier block valid | not checked | checked, from genesis |
| blocks above Hc | proof of work | proof of work only (the same) |
| the chain is the network's (most work) | the caller compares the tip | the caller compares the tip (`--expect`, `--max-tip-age`) |
| no reorg after tip X | not known | not known |

It does **not** add:

- **finality or recency.** The statement is "as of tip X with work W". A reorg deeper than the burial after X makes
  it stale, and the proof cannot show that. A caller compares X with its own node or several sources (`--expect`) and
  requires a fresh tip (`--max-tip-age`, the tip's block time against the caller's clock).
- **validation above Hc.** Confirmations above the core tip are proof of work only; `--min-validated-depth` sets how
  many must be fully validated. Until the core proof reaches the tip, a new deposit's block is above Hc.
- **an independent check of Bitcoin Core.** The rules are Core v28.0's code in the guest (CONSENSUS_RULES.md: 59
  rules the same, 3 stricter, 0 weaker); a consensus bug in that code is in the proof too.
- **trust-free cryptography.** It rests on RISC Zero 3.1.0's soundness and on the image ids being the reviewed source
  (reproducible builds, `reproduce/LIBZYNC_IDS`).

## 4. Trust table

| part | trusted for | how a verifier limits it |
|---|---|---|
| RISC Zero 3.1.0 (STARK, recursion) | the receipts mean what the guests ran | pinned version; dev-mode receipts and `RISC0_DEV_MODE` refused |
| core image `46666b6f…` (or `8888d59c…`) | full validation 0..Hc | rebuild it (`reproduce/`), compare the id |
| header image `de54b94f…` | proof of work and header rules 0..Ht, the MMR | rebuild it, compare the id |
| the bundle builder, the path service, the node it read | nothing | every part is checked; a wrong part fails |
| the caller's clock and tip source | freshness and chain identity | the caller's own node or several sources |
| Groth16 wrap (optional, §6) | the wrap is of the stated receipt | a trusted setup (below) and BN254 pairing assumptions |

## 5. Using it

```sh
# build (any machine; the core run's history.bin for the block-forest leaf, a node or the raw block)
zync-deposit --core core.rcpt --history history.bin --headers headers.rcpt --headers-file headers.txt \
             --height 170 --txid f4184fc5…9e16 --bitcoin-cli "bitcoin-cli" -o deposit.json
# check (the operator; every expectation is the caller's)
swiftsync-verify deposit --txid f4184fc5…9e16 --vout 0 --amount 1000000000 --script 4104ae1a…ac \
             --min-depth 6 --min-validated-depth 6 --expect <height>:<hash> --max-tip-age 7200 deposit.json
```

`--op-return <hex>`: an output (at `--vout`, or any) that is OP_RETURN followed by data pushes only (OP_0, 1-75 byte
pushes, OP_PUSHDATA1/2/4), their data concatenated equal to the payload. Any other script (OP_1..OP_16 included) does
not match; `--script` checks a scriptPubKey byte for byte. `zync-deposit` checks what it built with the verifier and
refuses to write a bundle that does not verify. `zync-deposit mmr-path <headers file> <tip> <height>` prints an MMR path.

The bundle (JSON, version 1): `core_receipt`, `header_receipts` (one or a chain of runs from genesis),
`core_tip_mmr_path`, and `block`: `height`, `header`, `forest` (`state`, `undo`, `path`) for a block at or below Hc or
`mmr_path` above it, `tx`, `index`, `branch`, `coinbase`, `coinbase_branch`. Hashes are hex in internal byte order,
receipts hex of their bincode bytes.

## 6. A contract chain

**Default: the STARK receipts, verified off chain** by the operator, relayer or signer set (CLI or WASM). No trusted
setup. Measured below: about 60 ms for a bundle with both receipts.

**Optional, on chain: Groth16.** A contract verifies one Groth16 proof with RISC Zero's verifier contract and reads
the journal. The steps: prove the composed receipt (§2 (b)), wrap it (`ProverOpts::groth16()`; r0vm runs
`identity_p254`, then the Docker prover `risczero/risc0-groth16-prover`), deploy or reuse RISC Zero's Groth16 verifier,
and have the contract check the image id, the journal's txid, output, depths and tip. Groth16 needs a trusted setup:
RISC Zero's is a ceremony for its one fixed STARK-verifier circuit, so it is universal across programs (one setup for
every image id) and holds if at least one participant was honest. The wrap also moves the assumption from hashes to
BN254 pairings (not post-quantum). The owner's default stays STARK; Groth16 is an add-on.

Measured end to end on block 170 (§7): the composed receipt in 293.4 s on a shared CPU box, its Groth16 wrap in
88.5 s, 3,129 bytes with a 256-byte seal. The wrap is not specific to this zyncAPP: the core repository already has
`host libzync-snark-wrap`. Separately, the deposit verifier accepts Groth16-wrapped core and header receipts unchanged (a chain proof may be succinct or
Groth16), so a bundle of the two wrapped receipts is the same statement in 22 KB.

## 7. Measured (15.11.2026)

Laptop: WSL2, 16 threads. hazync-coord: x86, 16 cores, 62 GB, shared (load 11-24 from other jobs during the runs).

| what | number |
|---|---|
| bundle, core 0..200 only (STARK, hex) | 456,550 bytes |
| bundle, core 0..200 + header chain 0..975,278 (STARK, hex) | 912,772 bytes |
| bundle, the same two receipts Groth16-wrapped | 21,856 bytes |
| `swiftsync-verify deposit`, core only / core + headers / Groth16 pair (laptop, wall) | 30 ms / 61 ms / 16 ms |
| `zync-deposit`, reading 975,279 headers for the MMR path (laptop) | 2.7 s |
| composed guest (b), executed | 816,427 cycles, 1 segment, journal 1,316 bytes |
| composed guest (b), proved succinct on hazync-coord's CPU | 293.4 s, 225,858 bytes |
| composed receipt, Groth16-wrapped | 88.5 s; 3,129 bytes (seal 256 bytes); verify 9 ms |
| Groth16 wrap of the core receipt 0..200 (hazync-coord CPU + Docker) | 94.4 s; 227,362 -> 4,633 bytes (seal 256 bytes); verify 5 ms |
| Groth16 wrap of the header receipt 0..975,278 | 97.0 s; 227,474 -> 4,745 bytes; verify 6 ms |
| WASM verifier with the deposit entry | 1,302,725 bytes (381,955 gzipped) |

Tests: 16 unit tests (`deposit::tests`), `test_deposit.sh` 22/22, `test_deposit_wasm.mjs` 10/10. Refusals covered: the
wrong txid (both ways), the wrong output, amount or script, insufficient depth or validated depth, a block not in
the chain (other height, tampered coin state, path or header), a tampered merkle or coinbase branch, the 64-byte
transaction trick on block 170 (whose merkle root IS dsha(coinbase txid || T): the 64 bytes with an empty branch
reach the root), a block above the core tip (refused by default, flagged when allowed), a dev-mode receipt (in the
bundle and in the environment), a flipped core receipt, a broken binding path, a header chain not from genesis or
below the core tip, the wrong network, tip identity and freshness, and an OP_RETURN payload mismatch.

The real receipts used: core proof of mainnet 0..200 (`8888d59c`, `swiftsync-verify/vectors/`, with its run's
`history.bin`), header proofs 0..100 + 101..200 and 0..975,278 (`de54b94f`), block 170 (the coinbase and Satoshi to
Hal Finney), block 800,000 (a segwit transaction; the coinbase's witness-commitment OP_RETURN). No v0.26.0
(`46666b6f`) mainnet core receipt is in the repository yet; its block leaves add the undo hash, which `forest.undo` carries.

## 8. Open

- A v0.26.0 core receipt: rerun the tests on one when the board has one, with its `history.bin`.
- Where a bridge gets `history.bin`-derived openings at mainnet scale (a path service, like `zync-serve` for coins).
- (b) as a published image, if a contract chain wants on-chain verification: review, a reproducible id, a chain of
  header runs (today one run from genesis), and a contract that reads its journal.
- The default policy for `--min-validated-depth` (today 0: the deposit's block itself must be fully validated, its
  burial may be proof of work only).
