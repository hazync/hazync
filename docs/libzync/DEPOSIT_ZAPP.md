# The deposit-proof zyncAPP

Status (15.11.2026): built on branch `feat/zapp-deposit`, tested on real receipts, not released. No new image id: it
checks two proofs that already exist. Code: the zAPP's own module `zapps/deposit` (ZYNCAPPS.md §9), on the core verifier
and the header-chain zAPP (`zapps/headers`) as libraries: `src/lib.rs` (verifier), `zapp-deposit` (CLI), `wasm/`
(`verify_deposit` / `verifyDeposit` in `zapp-deposit.js`), `zapp-deposit build` (bundle builder). Tests: `src/tests.rs`,
`test_deposit.sh` and `wasm/test-deposit.mjs`, all run by `zapps/deposit/test.sh`.

## 1. The statement

One file (a *deposit bundle*) that a bridge, rollup or other chain's operator checks, with its own question, to learn:

> transaction T (and, if asked, its output v holding this amount and scriptPubKey, or an OP_RETURN output carrying
> this payload) is at position i of block h; there are N blocks on top of it as of tip X (height, hash, chainwork W);
> every block from genesis to the core tip Hc was fully validated (scripts and signatures, by Bitcoin Core's consensus
> code compiled into the guest); blocks Hc+1..X are checked for proof of work and Core's header rules only.

The caller supplies T, v, the amount, the script or payload, the depths, the network and the image ids. The bundle
carries no txid and no image id. The raw transaction must hash to the caller's T (the header zAPP review's rule:
an answer that is a valid proof for another transaction is refused).

**What to gate on: `accepted`** (the CLI's exit 0, the WASM's `accepted`), and nothing else. It is true only when the
verdict is `OK`: every check passed, the tip proof has Core's `nMinimumChainWork`, and the tip was compared with a
source the caller trusts (`--expect`, `--mempool` or `--bitcoin-cli`; `require_identity`, on by default). The other
verdicts:

- `DEMO` (exit 2): the checks passed, but the chain is below the minimum chain work, or, for a core proof alone, below
  the checkpoint its image pins (block 970,000), or the tip was not compared. **A DEMO statement is cheap to forge**:
  anyone can mine a difficulty-1 fork from genesis and prove it, and it passes the same checks. So the statement is
  withheld (null in WASM, not printed by the CLI) unless the caller asks for it (`--allow-demo`, `"allow_demo": true`),
  and it is never `accepted`.
- `FAIL` (exit 1): a check failed; no statement.

A caller that compares the tip by other means can turn the identity requirement off (`--no-require-identity`,
`"require_identity": false`); then `OK` and `accepted` need only the checks and the work, and saying so is its choice.

**Whose work counts.** "The tip proof" is the header-chain proof when header receipts are given (and bound, §2), else
the core proof. With headers, `OK` does NOT also require the core proof's own verdict to be `OK`: a core proof that is
`DEMO` on its own (its chain 0..Hc below the minimum chain work, or below its pinned checkpoint, as every core proof is
while the board re-proves from genesis) gives `OK` when the bound header chain has the minimum chain work and its tip
matched the caller's source. This is a choice, and I believe a sound one: the binding makes the core chain 0..Hc the
header chain's own first Hc+1 blocks (§2), so it is a prefix of a chain with the network's work whose tip the caller
recognised, and a cheap fork of it would have to be a fork of that chain below Hc, which the binding refuses. What the
core proof adds is validation of 0..Hc, not work; its low work alone is no sign of a forgery once it is bound. Without
headers the core proof is the tip proof, and its `DEMO` stays `DEMO`.

## 2. How the transaction is bound to a fully validated chain

Two proofs exist:

- the **core proof** (`46666b6f…`, v0.26.0; its proofs from the earlier `8888d59c…` still verify by id): every block
  0..Hc fully validated. Its `ChainState` holds a block forest with one leaf per block: the block hash, the coin
  state after it and (from v0.26.0) its undo hash. It reaches Hc, which today is well below the tip (the board is
  re-proving from genesis).
- the **header-chain proof** (`de54b94f…`): proof of work and Core's header rules 0..Ht, with an MMR of every block
  hash. It follows the tip.

I considered two ways to join them.

**(a) Composition in the verifier (my choice).** The verifier checks both receipts, then:

1. *binding*: the core tip hash is the header MMR's leaf at height Hc (one MMR path). Both are hash chains from
   genesis, so the header chain's blocks 0..Hc are exactly the core's (a difference would be a SHA-256d collision);
2. *the block*: block h (h <= Hc) is a leaf of the core block forest (header, coin state, undo hash, path), so it is
   one of the fully validated blocks;
3. *the transaction*: it is at position i of that block by its merkle branch, with the tree depth pinned by the
   block's coinbase and its branch. What closes the 64-byte ambiguity (an inner merkle node is 64 bytes, so a 64-byte
   transaction cannot be told apart from one; CVE-2012-2459's family; BIP 54 would forbid it) is the transaction's
   serialisation WITHOUT witness (the txid preimage) not being 64 bytes, together with that coinbase depth pin; a
   given form of exactly 64 bytes is also refused early. The outputs are parsed from the bytes that hash to T;
4. *depth*: N = Ht - h + 1 (Hc - h + 1 without a header proof), of which Hc - h + 1 are fully validated.

A block above Hc can be shown only by the header MMR: proof of work only. It is refused unless the caller passes
`--allow-pow-only-block` AND `--min-validated-depth 0`, and the statement then says so.

`--min-validated-depth` defaults to **1**: the deposit's own block h is fully validated (h <= Hc); its burial may be
proof of work only. Validating the burial adds next to nothing: a valid (say, empty) block on top costs a miner the
same work as an invalid one, so burial blocks are bought with work either way. Full validation's job is to rule out a
fake deposit, a transaction in an invalid block (spending coins that do not exist, a bad signature), and that needs
block h itself, not the blocks above it. A caller may still ask for more than 1. An explicit 0 is accepted: on its own
it changes nothing (a block above Hc is still refused unless `--allow-pow-only-block`); it is the second of the two
explicit choices that accept a proof-of-work-only deposit block.

**(b) A composed receipt.** A small guest verifies the core and header receipts as assumptions (`env::verify`),
makes the same four checks and commits one statement. Built to measure it
(`experimental/libzync-bench/deposit/tools/`, not a published image; raw output in `RESULT_measured.txt`): **816,427 cycles, one segment**, journal 1,316
bytes, on the real bundle (core 0..200, header chain 0..975,278, block 170). Proved on hazync-coord's CPU through r0vm
3.1.0 in 293.4 s (a 225,858-byte succinct receipt), wrapped in Groth16 in a further 88.5 s (3,129 bytes). A
tampered branch makes the guest panic. Its guest (laptop build, `184e81af…`) is not reproducible as built: a
published version would go through `reproduce/` like the others.

**Why I chose (a).** It adds no image id to review, reproduce and pin (the trust is the two images already published), no
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
| the output is still unspent at tip X | not known | **not shown** either (below) |

It does **not** add:

- **that the output is still unspent.** The bundle shows the deposit was made (the output was created in block h); it
  says nothing about whether it was spent later, at or before tip X. A bridge that needs "still unspent" uses the coin
  liveness zAPP (`zapps/liveness`, branch `feat/zapp-liveness`), which answers it against the core proof's coin log.
- **anything, at DEMO.** A DEMO statement can be shown by a cheap fork (§1); only `accepted` means something.

- **finality or recency.** The statement is "as of tip X with work W". A reorg deeper than the burial after X makes
  it stale, and the proof cannot show that. A caller compares X with its own node or several sources (`--expect`) and
  requires a fresh tip (`--max-tip-age`, the tip's block time against the caller's clock).
- **validation above Hc.** Confirmations above the core tip are proof of work only; `--min-validated-depth` sets how
  many must be fully validated (default 1: the deposit's block itself, §2). Until the core proof reaches the tip, a new deposit's block is above Hc.
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
zapp-deposit build --core core.rcpt --history history.bin --headers headers.rcpt --headers-file headers.txt \
             --height 170 --txid f4184fc5…9e16 --bitcoin-cli "bitcoin-cli" -o deposit.json
# check (the operator; every expectation is the caller's)
zapp-deposit --txid f4184fc5…9e16 --vout 0 --amount 1000000000 --script 4104ae1a…ac \
             --min-depth 6 --min-validated-depth 6 --expect <height>:<hash> --max-tip-age 7200 deposit.json
```

`--op-return <hex>`: an output (at `--vout`, or any) that is OP_RETURN followed by data pushes only (OP_0, 1-75 byte
pushes, OP_PUSHDATA1/2/4), their data concatenated equal to the payload. Any other script (OP_1..OP_16 included) does
not match. The concatenation loses the push boundaries and which push opcode carried each part (one push `abcd` and
two pushes `ab` `cd` carry the same payload); where that matters, I recommend `--script`, which checks the scriptPubKey
byte for byte. `--allow-demo` prints a DEMO statement (never accepted); `--no-require-identity` drops the tip-source
requirement (§1). `zapp-deposit build` checks what it built with the verifier and
refuses to write a bundle that does not verify. `zapp-deposit mmr-path <headers file> <tip> <height>` prints an MMR path.

The bundle (JSON, version 1): `core_receipt`, `header_receipts` (one or a chain of runs from genesis),
`core_tip_mmr_path`, and `block`: `height`, `header`, `forest` (`state`, `undo`, `path`) for a block at or below Hc or
`mmr_path` above it, `tx`, `index`, `branch`, `coinbase`, `coinbase_branch`. Hashes are hex in internal byte order,
receipts hex of their bincode bytes. Since the review of 18.11.2026 the verifier refuses a field the format does not
name (at any level), and a block carries exactly the opening its height takes: `forest` at or below Hc, `mmr_path`
above it, never both (`zapp-deposit build` never wrote either form, so no bundle it made changes meaning).

In WASM the ask is a JSON object with the keys of `zapp-deposit.js`'s doc comment; any other key is refused (a FAIL
naming it), as the CLI refuses an unknown flag: a misspelt constraint (`ammount`, `min_dept`) would otherwise be
dropped with its check. A key given as null is refused too, never read as absent: JSON.stringify writes NaN and
Infinity as null, so `amount: parseInt('ten btc')` would otherwise drop the amount check. Leave a key out (undefined)
for its default. The WASM's `now` is the page's clock (the glue passes the device's unless given one): the
module has no clock of its own to clamp it to, as the CLI clamps `--now`, so a page that passes its own `now` is
trusted for it (§4, the caller's clock).

Above Hc the transaction's inclusion is checked by the header zAPP's transaction parser (`verify_tx_in_header`) and
its outputs are read by libzync's (the one the fully validated path uses). The outputs are bound to the txid by
libzync's own stripped serialisation (a check of its own), so the binding does not rest on the two parsers agreeing;
a unit test pins that they agree on every real transaction in the tests, in both serialisations.

With `--mempool` or `--bitcoin-cli` the CLI verifies the bundle twice: once to learn the tip to ask the sources
about, once with their answer. That is cost only (about 60 ms each with both receipts, §7).

## 6. A contract chain

**Default: the STARK receipts, verified off chain** by the operator, relayer or signer set (CLI or WASM). No trusted
setup. Measured below: about 60 ms for a bundle with both receipts.

**Optional, on chain: Groth16.** A contract verifies one Groth16 proof with RISC Zero's verifier contract and reads
the journal. The steps: prove the composed receipt (§2 (b)), wrap it (`ProverOpts::groth16()`; r0vm runs
`identity_p254`, then the Docker prover `risczero/risc0-groth16-prover`), deploy or reuse RISC Zero's Groth16 verifier,
and have the contract check the image id, the journal's txid, output, depths and tip. Groth16 needs a trusted setup:
RISC Zero's is a ceremony for its one fixed STARK-verifier circuit, so it is universal across programs (one setup for
every image id) and holds if at least one participant was honest. The wrap also moves the assumption from hashes to
BN254 pairings (not post-quantum). The default stays STARK, with no trusted setup; Groth16 is an add-on.

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
| `zapp-deposit`, core only / core + headers / Groth16 pair (laptop, wall) | 30 ms / 61 ms / 16 ms |
| `zapp-deposit build`, reading 975,279 headers for the MMR path (laptop) | 2.7 s |
| composed guest (b), executed | 816,427 cycles, 1 segment, journal 1,316 bytes |
| composed guest (b), proved succinct on hazync-coord's CPU | 293.4 s, 225,858 bytes |
| composed receipt, Groth16-wrapped | 88.5 s; 3,129 bytes (seal 256 bytes); verify 9 ms |
| Groth16 wrap of the core receipt 0..200 (hazync-coord CPU + Docker) | 94.4 s; 227,362 -> 4,633 bytes (seal 256 bytes); verify 5 ms |
| Groth16 wrap of the header receipt 0..975,278 | 97.0 s; 227,474 -> 4,745 bytes; verify 6 ms |
| the deposit zAPP's own WASM module (`zapps/deposit/wasm`, sha256 `f7d7f49b…` (before it `c0294e79…`) (before it `e5bb6c30…`) (before it `5f1e1fb1…`); its CLI is the crate `zapps/deposit/cli`, the only one linking zapps/common) | 1,197,668 bytes (343,230 gzipped) |

Tests: 23 unit tests (`zapps/deposit/src/tests.rs`), `test_deposit.sh` 32/32 (5 chain-identity cases: an agreeing node, one
naming another block, one giving no chainwork, one giving another chainwork, and the tip given), `wasm/test-deposit.mjs`
24/24. Also covered since the review of 15.11.2026: `accepted` and the verdict under each of `require_identity` and
`allow_demo`, a DEMO statement withheld, a transaction in its witness serialisation in a fully validated block, and a
block of the v0.26.0 image (`46666b6f`, whose leaves commit the undo hash) on its real receipt of blocks 0..1. Refusals covered: the
wrong txid (both ways), the wrong output, amount or script, insufficient depth or validated depth (the default 1 on a block above the core tip, 2 at h == Hc), a block not in
the chain (other height, tampered coin state, path or header), a tampered merkle or coinbase branch, the 64-byte
transaction trick on block 170 (whose merkle root IS dsha(coinbase txid || T): the 64 bytes with an empty branch
reach the root), a block above the core tip (refused by default, flagged when allowed), a dev-mode receipt (in the
bundle and in the environment), a flipped core receipt, a broken binding path, a header chain not from genesis or
below the core tip, the wrong network, tip identity and freshness, and an OP_RETURN payload mismatch; since the review
of 18.11.2026 an unknown bundle field, the other opening for the block's height, and (WASM) a misspelt or null ask key.

The real receipts used: core proof of mainnet 0..200 (`8888d59c`, `swiftsync-verify/vectors/`, with its run's
`history.bin`), header proofs 0..100 + 101..200 and 0..975,278 (`de54b94f`), block 170 (the coinbase and Satoshi to
Hal Finney), block 800,000 (a segwit transaction; the coinbase's witness-commitment OP_RETURN), and block 1 under the
v0.26.0 image (`46666b6f`, `experimental/libzync-bench/zapp/mainnet_1_1_46666b6f.rcpt`: the undo-hash leaf, its opening
made without a history.bin). A v0.26.0 receipt deep enough to put a block with spends under the core tip waits on the
first v0.26.0 board run.

## 8. Open

- A deeper v0.26.0 core receipt (a block with spends, so a non-empty undo hash): rerun the tests on the board's first
  v0.26.0 run, with its `history.bin`.
- Where a bridge gets `history.bin`-derived openings at mainnet scale (a path service, like `zync-serve` for coins).
- (b) as a published image, if a contract chain wants on-chain verification: review, a reproducible id, a chain of
  header runs (today one run from genesis), and a contract that reads its journal.
