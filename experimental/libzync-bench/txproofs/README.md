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

## 5. Open questions, to measure before building

- **Head start:** what share of a block was in the mempool, and in the node's own template, 30, 60, 120 and
  300 s before the block (`headstart.py`, read-only on the bridge node). That is the share of the tip work that
  moves out of the block's critical path.
- **Waste:** transactions proved but never mined (replaced, evicted). Pre-proving only what the template would
  mine reduces it.
- **Soft forks:** a validity batch is proved under one set of script flags. A flag change is a handover, so
  batches from before it are not reused after it.
- **Unconfirmed parents:** a transaction spending a mempool parent is provable (the parent's outputs are known);
  its spent leaf is finished when both are mined, in the same block or not.
