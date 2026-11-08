# Proving transactions apart from blocks

Status: design and measurements. Nothing here is built into the guest. ⛔ EXPERIMENT ONLY.

## 1. The split the coin log makes possible

A block proof does two different jobs:

| job | depends on | cost on 962,000 (r17m + coin log) |
|---|---|---|
| **validity**: every script and signature passes | the transaction and the coins it spends (value, scriptPubKey), and the block's script flags | the chunks, ~365 M of ~438 M |
| **state**: every spent coin exists and is unspent, the log moves on, the header chain links | the coin log, the block | the aggregate, 73.1 M |

Validity does not depend on the UTXO set. Today's chunks already take the spent coins' data from the host and
commit what they assumed (`tx_bind`). The only state check is "these coins are unspent", which the aggregate makes
against the coin log. Height, MTP and the coinbase flag of a spent coin are not used by any script check: they only
enter the coin's leaf, and CLTV/CSV opcodes compare the transaction's own fields.

So the validity of a transaction can be proved **before** its block exists, as soon as it reaches the mempool,
and the block proof only does the state job. Design B could not do this: its forest is the UTXO set only at H, so
"these inputs are unspent now" had no meaning between closures. The coin log is sound at every block.

## 2. The design: a transaction log

The coin log idea applied to transactions:

1. **Validity batches.** The prover takes mempool transactions as they arrive (every N seconds, or every K
   signatures) and proves a batch. ⛔ A batch, never one proof per transaction: one proof per transaction loses
   the batch signature check, 24.2k cycles per ECDSA signature at 438 signatures and 16.2k at 50k, against 83.8k
   alone (measured, `project_libzync_batch_verify_measured`): 3.4x to 5.1x.
2. **Records.** For each transaction the batch commits a record: txid, wtxid, outpoints spent, and, for every
   spent coin and every output, the coin leaf's prefix (UTREEXO_FORMAT.md §2: tag, txid, vout, value,
   scriptPubKey) as a SHA-256 midstate plus its tail bytes. The leaf's suffix (height, flags, creation MTP) is
   only known when the parent and the child are mined. Finishing a leaf at block time is one or two compressions.
   **The leaf format does not change.**
3. **The log.** A rolling proof appends each batch's records to an append-only stump forest, the coin log's
   structure. One receipt covers every transaction ever pre-proved.
4. **The block step** is the aggregate, plus: open each block transaction's record in the transaction log (one
   batched path, as the coin log's `update`), finish the leaves, flip the spent coins, append the new ones.
   Transactions the prover never saw (miners' private ones) and the coinbase go through an ordinary chunk at block
   time.

At the tip, after a block arrives, the remaining work is about the aggregate (~73 M on 962,000) plus the opening
and the unseen transactions, instead of ~438 M. ⚠ Derived from measured parts, not measured as a whole: the
opening costs about log2(records) hashes per transaction, 100 to 525 cycles each (coin-log bench), so 2k to 10k
cycles per transaction against ~100k of script work.

## 3. What else falls out

- **Ordering and chunk shape no longer matter for cost.** A validity batch can be as large as the signature
  check wants, across many blocks during IBD: the batch check flattens at ~16k cycles per signature past 50k
  signatures, and keys repeat. Measured over 144 mainnet blocks to 974,290 (`keyreuse.py`): 1,000,690
  single-key signatures, distinct keys per signature 0.572 in one block, 0.550 over 6, 0.523 over 36, 0.492
  over 144. A key's setup is ~14.5k cycles every time it appears in a batch.
- **Reorgs are cheap.** Validity proofs do not depend on which block a transaction lands in. A reorg re-runs only
  the state steps.
- **IBD in any order.** The whole chain's transactions can be proved in parallel and in any order, by anyone.
  Only the state chain (the aggregate, block by block) is sequential.
- **Zero-conf zyncAPP.** "Transaction T is script-valid, and every coin it spends was unspent at tip T" is a
  record in the transaction log plus unspent leaves in the coin log at the same height. A merchant or a light
  wallet gets it before the transaction confirms. Only possible with a log that is sound at every block.
- **Ghost: proof-ready blocks.** A pool knows its template's transactions before it finds a block. With them
  pre-proved, a block found by the pool needs only the state step, so it can carry its proof within about the
  aggregate's proving time.

## 4. Rejected

- **One proof per transaction** (wallet-made or prover-made): loses batching, 3.4x to 5.1x more signature work.
  A wallet attaching its own proof is also a new piece of software for every wallet.
- **Optimistic proofs** (assume valid, prove only if challenged): not a validity proof. The product is "the
  soundness of Core".

## 5. Measured: the head start (8 blocks, 974,293..974,300, 11-02)

`headstart.py`, read-only on the bridge node, polling the mempool every 5 s and the node's template every 30 s.
Share of each block's **inputs** (signature work) that had been in the mempool at least so long before the block:

| block | txs | ≥ 0 s | ≥ 60 s | ≥ 120 s | ≥ 300 s | never seen (inputs) |
|---|---|---|---|---|---|---|
| 974,293 | 2,147 | 100% | 97.7% | 91.3% | 70.7% | 0 |
| 974,294 | 3,291 | 100% | 98.2% | 94.4% | 75.3% | 0 |
| 974,295 | 3,111 | 99.9% | 95.3% | 91.2% | 69.8% | 8 |
| 974,296 | 1,350 | 99.3% | 96.0% | 87.1% | 77.9% | 70 |
| 974,297 | 1,164 | 100% | 78.5% | 25.3% | 22.4% | 0 |
| 974,298 | 4,780 | 98.8% | 94.9% | 81.1% | 59.3% | 70 |
| 974,299 | 3,420 | 98.8% | 95.4% | 81.6% | 38.5% | 71 |
| 974,300 | 3,136 | 100% | 93.4% | 85.9% | 59.4% | 0 |

- **Nearly everything is seen first:** 98.8 to 100% of inputs were in the mempool before the block; at most 71 inputs
  per block were not (miners' own transactions). Those are the block-time chunk.
- **A minute is usually enough:** 93 to 98% of inputs had at least 60 s, except 974,297, a block found shortly after
  the one before it, which mined mostly fresh arrivals (78.5% at 60 s, 25% at 2 min). The prover must keep up with
  arrivals within about a minute, not prove a backlog.
- **The template is no better a filter:** the node's own template held about the same shares, slightly lower, so
  proving the whole mempool as it arrives is the simpler policy.
- **Waste:** 282 of 21,828 transactions that arrived during the 68 minutes left the mempool unmined (1.3%: RBF
  replacements and evictions together, an upper bound for RBF).

## 6. Open questions, to measure before building

- **Prover throughput:** proving the mempool's arrivals within a minute, in batches, on how many cards.
- **Soft forks:** a validity batch is proved under one set of script flags. A flag change is a handover, so
  batches from before it are not reused after it.
- **Unconfirmed parents:** a transaction spending a mempool parent is provable (the parent's outputs are known);
  its spent leaf is finished when both are mined, in the same block or not.

## 7. Build plan (11-03): two phases

### Phase A — speculative chunks (small guest change)

A chunk's journal is tied to its block by (height, block hash, MTP, block time). For the NEXT block H = tip + 1, three
of those are known in advance and the fourth does not matter:

- **height** H and **MTP** (the median of the tip's last 11 timestamps) are fixed once the tip is;
- **block time** enters a chunk only as the lock time before CSV (`lock_time = mtp` from `NET.csv_height` on);
- **block hash** enters only through `block_script_flags`, whose only hash-dependent cases are Core's two
  script-flag exception blocks (2010-2012).

So a chunk proved at arrival "for height H with MTP m" is exactly the chunk the block-time prover would make, if the
transaction lands in block H. The change:

1. A chunk may commit a **speculative context**: block hash and time zero, flag bit "speculative". Allowed only when
   H >= `NET.csv_height` (so block time is unused) and H is not a flag-exception height.
2. The aggregate accepts a speculative chunk when its height and MTP equal the block's (the hash and time checks are
   skipped only for such chunks, and only under that height rule).
3. **Record selection.** A batch holds mempool transactions in arrival order, some never mined. The host gives the
   aggregate the block's order as (journal, record) pairs; the aggregate walks records in that order. Unselected
   records are ignored (they change no state); the Merkle root over the walked txids still binds the exact list.
   A record selected twice is refused.
4. **One invalid transaction spoils its chunk.** A chunk with any invalid record commits `all_valid = 0`, and the
   aggregate refuses every chunk it is given that is not all valid, selected records or not (review 11-03). The host
   must prove a batch only of transactions it has checked (Core's mempool acceptance) and, if a batch chunk still
   fails, drop the failing transaction and re-prove that batch before the block uses it.
5. Transactions not pre-proved (miners' own, late arrivals, the coinbase) go through ordinary chunks at block time.

Cost of being wrong: a transaction that misses block H is re-proved for H + 1. Waste = the backlog that survives each
block (measure: `headstart.py` extended to count it).

### Phase B — context-free validity records (bigger guest change)

If Phase A's waste is large (a congested mempool re-proves its backlog every block), split the chunk: a validity
batch proves scripts, CheckTransaction, fee, weight and sigops with no block context (records as §2), and the aggregate
does the block-time rules itself (IsFinalTx, BIP68, maturity, the leaf suffixes) from small per-input facts. That moves
Core's `SequenceLocks`/`IsFinalTx` logic into the aggregate, so it needs its own review round.

## 8. Measured: Phase A's waste (8 blocks, 974,421..974,428, 11-03, `template_waste.py`)

The last block template seen before each block (polled every 15 s, read-only on the bridge node), by input count:

| block | template inputs | mined (useful) | not mined (re-proved next block) | waste | block txs not in the template |
|---|---|---|---|---|---|
| 974,421 | 7,670 | 7,467 | 203 | 2.6% | 183 |
| 974,422 | 8,556 | 8,176 | 380 | 4.4% | 4 |
| 974,423 | 7,709 | 6,894 | 815 | 10.6% | 632 |
| 974,424 | 8,012 | 7,714 | 298 | 3.7% | 279 |
| 974,425 | 7,816 | 6,119 | 1,697 | 21.7% | 1,614 |
| 974,426 | 8,025 | 7,256 | 769 | 9.6% | 777 |
| 974,427 | 5,232 | 5,049 | 183 | 3.5% | 1,762 |
| 974,428 | 7,955 | 7,711 | 244 | 3.1% | 173 |
| **8 blocks** | 60,975 | **56,386** | **4,589** | **7.5%** | |

- **Waste is small: 7.5% of the pre-proved inputs**, 2.6-21.7% per block. Phase B (context-free records) is not needed
  for this: re-proving the template's leftovers for the next height costs well under a tenth of the work.
- **The block-time chunk varies a lot** (4 to 1,762 transactions not in the last template). Large values follow
  templates that lagged the mempool (974,427 came 96 s after its parent, from a 4,678-tx template). A prover should
  refresh its template more often than 15 s and prove arrivals continuously; the count above is transactions, not
  inputs.
