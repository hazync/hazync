# zyncAPPs: building on the core proof

Status: design. No zyncAPP is built yet. The core facts in §2 exist in `ChainState` version 2 (SWIFTSYNC_PROOFS.md §3,
HANDOVER.md §2); the shared check of §4 is the code every zyncAPP must carry and is not written yet.

## 1. Core and zyncAPPs

- **The core** proves one statement: the chain ending at `tip_hash` is valid Bitcoin under Core's rules, from
  genesis to `height`. It is public, free and as small as possible. Its output is `ChainState`.
- **A zyncAPP** is a separate small guest. It verifies a core receipt inside the zkVM and answers one question with
  it: "these coins exist at H and I control them", "this payment is in the chain", "the UTXO set holds X sat".
  It has its own image id and its own receipt. A zyncAPP never changes the core.

## 2. What a zyncAPP can use (ChainState version 2)

| fact | fields | lets a zyncAPP show |
|---|---|---|
| which chain, how much work | `tip_hash`, `height`, `cum_work`, `recent_times`, `max_time` | the tip, its height and chainwork, time context |
| every spendable coin that outlived its block, and who spent it (coin log; not coins spent in their own block, not OP_RETURN outputs, COINLOG.md §6) | `utxo_roots`, `utxo_leaves` (COINLOG.md) | a coin is unspent now, or was spent by (height, tx, input); amount, scriptPubKey, height, coinbase flag, creation MTP; by one path |
| every coin unspent at H (design B) | `utxo_roots`, `utxo_leaves` (UTREEXO_FORMAT.md) | a coin exists at H, by an inclusion path |
| total amount of the live coins | `utxo_value` | the UTXO set's total, with no walk over the coins |
| every block in the chain, and the coin state after it | `block_roots`, `block_leaves` (HANDOVER.md §3) | a block at a height is in this chain; with a Merkle branch, any transaction; with the coin log, any coin's state at that height |
| what was checked | `profile` | network; scripts and signatures checked or skipped |
| where it started | `anchor_id` | genesis-anchored (or not) |
| SwiftSync closed at H | `ss_set`, `ss_hints` | the coin forest really is the UTXO set at H |
| which images proved it | `self_id`, `lineage` | the image that made the receipt and the handovers behind it |

With the **coin log** (profile bit 4), coin facts hold at every block: a zyncAPP about coins answers at the tip, or at
any earlier height through the block forest. ⚠ Under **design B** the coin forest is the UTXO set **only** in a proof
closed at H (`ss_set == 0` and `ss_hints` equal to the published hints file's hash), so a zyncAPP answers "at H".

## 3. When a zyncAPP can ship

- **Any time,** if it uses only facts the core already commits (§2). No core change, no handover, no new core id.
- **At the next handover,** if it needs a fact the core does not commit. The fact is appended to `ChainState` and
  `version` goes up by one (HANDOVER.md §2). The handover is the core's release window: zyncAPPs that need new facts
  are queued for it. A soundness fix or a soft fork can force an earlier handover.
- Appending never breaks an existing zyncAPP: risc0's serde reads fields in order and ignores trailing words, so a
  zyncAPP built for version n decodes the prefix it knows from any later journal.

## 4. Template

A zyncAPP takes the **core image id as an input**, not compiled in. A compiled-in id would stop working at the core's
first handover, when the newest core id changes.

```
inputs:  core_id, core_journal, <the question's own inputs>
1. env::verify(core_id, core_journal)                  the receipt is a real core receipt of core_id
2. J = decode the ChainState prefix this zyncAPP knows (version >= the version it needs)
3. check_core_statement(J, core_id):                   the same function in every zyncAPP
     J.kind == KIND_CHAIN, J.version >= needed, J.self_id == core_id
     J.profile: the network the zyncAPP is for; scripts checked (bit 1 clear) unless it states otherwise
     coin log (profile bit 4), or J.ss_set == 0          design B: closed at H (for any question about coins)
4. answer the question from J and its own inputs only
5. commit: core_id, J.lineage, J.anchor_id, J.ss_hints, J.height, J.tip_hash, J.max_time, J.cum_work, the answer
```

`check_core_statement` exists because the core image id is not enough on its own: the `is_base` bug (LIBZYNC.md §7)
was a check the receipt's id could not catch. Every zyncAPP repeats the same checks, from one shared source file.

### What the zyncAPP's verifier checks

The zyncAPP's receipt verifies against the zyncAPP's own id. From its journal, the verifier then checks what the
zyncAPP could not know inside the zkVM, exactly as SWIFTSYNC_PROOFS.md §5 does for a core proof:

- `core_id` is a published core image, and `lineage` is that image's (§6 of HANDOVER.md). Which core images to trust
  is the verifier's decision, the same one it makes for a core proof.
- the zyncAPP receipt is a real proof: not a FakeReceipt, and `RISC0_DEV_MODE` not set in the verifier.
- `anchor_id` is the network's PINNED genesis anchor (SWIFTSYNC_PROOFS.md §5 step 3).
- `cum_work` is at least Core's `nMinimumChainWork` for the network (§5 step 3a); below it the answer is about a
  valid chain, but not shown to be the network's.
- design B only: `ss_hints` is the hash of the published hints file at `height`.
- `max_time <= now + 7200` (time-too-new).
- **Chain identity:** `tip_hash` is the network's block at `height`, from the verifier's node, a block explorer
  (mempool.space) or a known hash (SWIFTSYNC_PROOFS.md §5 step 7). Without it, the answer is about a valid chain,
  not necessarily the network's.

## 5. Publishing a zyncAPP

- **Source is public.** A user must be able to rebuild the zyncAPP and get the same image id, as `reproduce/` does for
  the core. A zyncAPP whose source is hidden cannot be trusted, whatever its receipt says.
- **Its id is published** next to the core ids, with the core versions it reads.
- **Nothing commercial in the guest.** Payment, accounts and rate limits live outside the proofs.

## 6. Examples

| zyncAPP | inputs | uses | answer |
|---|---|---|---|
| **zyncPOO** (proof of ownership) | coins with log paths; a challenge; a signature by each coin's key | `utxo_roots` | "these coins, worth X sat, are unspent at the tip (or at height h) and the holder signed the challenge". A private form commits only the total |
| coin history | a coin's log path, the spender record, block-forest paths | `utxo_roots`, `block_roots` | "coin C was created at h1 and spent by tx T at h2"; chained hop by hop, a coin's path back to its coinbase or forward to today |
| proof of payment | block header, the coin-state digest after that block (a block leaf commits it), the block-forest path, a Merkle branch to the transaction | `block_roots` | "transaction T is in this chain at height h" |
| supply | none | `utxo_value`, closed at H | "the UTXO set at H holds X sat" (compare with the issuance schedule) |
| snapshot check | a Core `dumptxoutset` | `utxo_roots`, `utxo_value` | "this snapshot is the UTXO set at H" (`utxo_snapshot_check.py` does it outside the zkVM today) |

## 7. Open

- `check_core_statement` as a crate shared by the verifier and every zyncAPP.
- Freshness: settled by the coin log (sound at every block). Under design B coins are "at H".
- Paths for old heights: the host keeps (or replays) the log at that height; a disk-backed store is needed on mainnet.
- Signature checks inside a zyncAPP should reuse the core's libsecp and script code, not a new implementation.

## 5. Coin facts and journeys (built, 11-03)

The shared checks are `libzync/src/facts.rs` (no_std, for zyncAPP guests and native verifiers). The host writes
`history.bin` (libzync/src/history.rs) on every run from genesis: per height the block hash, the live value, the
spends as (position, tx, vin) and the appended leaves. From it `zync-path` (swiftsync-verify crate) produces, and
`swiftsync-verify --coin / --journey` checks against a verified tip proof:

| file | proves |
|---|---|
| `CoinFact` (`zync-path coin <history> <tip> <at> <pos>`) | coin C at log position p is unspent, or was spent by (height, tx, vin), in the coin state after height `at`; below the tip, `at`'s block leaf is opened in the tip's block forest |
| `CoinJourney` (`zync-path journey <history> <tip> <pos> --bitcoin-cli "<cmd>"`) | the transaction that created C is at its position in a proven block (header + Merkle branch + block leaf), C's leaf is that output's (value, scriptPubKey, height, coinbase, tracked, creation MTP), C's state at the tip, and, if spent, the spending transaction is the one the log names and its input spends C |

Walking a journey backwards (a creating transaction's inputs) and forwards (the spender's outputs) is a chain of
these files. Limits:

- **A 64-byte transaction cannot be shown in a block** (`tx_in_block`): its txid could be an inner node of the Merkle
  tree. Coins created or spent by one have a coin fact but no journey.
- **Coins created and spent in the same block** never enter the log: that hop is shown by the block alone.
- `history.bin` and `zync-path` replay everything in memory; a mainnet path service needs the disk-backed store
  (ROADMAP C1).
