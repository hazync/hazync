# zyncAPPs: building on the core proof

Status: design. No zyncAPP is built yet. The core facts in §2 exist in `ChainState` v1 (SWIFTSYNC_PROOFS.md §3,
HANDOVER.md §2); the shared check of §4 is the code every zyncAPP must carry and is not written yet.

## 1. Core and zyncAPPs

- **The core** proves one statement: the chain ending at `tip_hash` is valid Bitcoin under Core's rules, from
  genesis to `height`. It is public, free and as small as possible. Its output is `ChainState`.
- **A zyncAPP** is a separate small guest. It verifies a core receipt inside the zkVM and answers one question with
  it: "these coins exist at H and I control them", "this payment is in the chain", "the UTXO set holds X sat".
  It has its own image id and its own receipt. A zyncAPP never changes the core.

## 2. What a zyncAPP can use (ChainState v1)

| fact | fields | lets a zyncAPP show |
|---|---|---|
| which chain, how much work | `tip_hash`, `height`, `cum_work`, `recent_times`, `max_time` | the tip, its height and chainwork, time context |
| every coin unspent at H | `utxo_roots`, `utxo_leaves` (UTREEXO_FORMAT.md) | a coin exists at H: amount, scriptPubKey, height, coinbase flag, creation MTP; by an inclusion path |
| total amount at H | `utxo_value` | the UTXO set's total, with no walk over the coins |
| every block in the chain | `block_roots`, `block_leaves` (HANDOVER.md §3) | a block at a height is in this chain; with a Merkle branch, any transaction, including spends and `OP_RETURN`s |
| what was checked | `profile` | network; scripts and signatures checked or skipped |
| where it started | `anchor_id` | genesis-anchored (or not) |
| SwiftSync closed at H | `ss_set`, `ss_hints` | the coin forest really is the UTXO set at H |
| which images proved it | `self_id`, `lineage` | the image that made the receipt and the handovers behind it |

⚠ The coin forest is the UTXO set **only** in a proof closed at H (`ss_set == 0` and `ss_hints` equal to the
published hints file's hash). Past H, at the tip, it is not (SWIFTSYNC_PROOFS.md §6a). A zyncAPP about coins answers
"at H", and is as fresh as the latest closed H.

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
     J.ss_set == 0                                      closed at H (for any question about coins)
4. answer the question from J and its own inputs only
5. commit: core_id, J.lineage, J.anchor_id, J.ss_hints, J.height, J.tip_hash, J.max_time, the answer
```

`check_core_statement` exists because the core image id is not enough on its own: the `is_base` bug (LIBZYNC.md §7)
was a check the receipt's id could not catch. Every zyncAPP repeats the same checks, from one shared source file.

### What the zyncAPP's verifier checks

The zyncAPP's receipt verifies against the zyncAPP's own id. From its journal, the verifier then checks what the
zyncAPP could not know inside the zkVM, exactly as SWIFTSYNC_PROOFS.md §5 does for a core proof:

- `core_id` is a published core image, and `lineage` is that image's (§6 of HANDOVER.md). Which core images to trust
  is the verifier's decision, the same one it makes for a core proof.
- `anchor_id` is the network's genesis anchor.
- `ss_hints` is the hash of the published hints file at `height`.
- `max_time <= now + 7200` (time-too-new) and `tip_hash` is on the chain the verifier sees, if it has a node.

## 5. Publishing a zyncAPP

- **Source is public.** A user must be able to rebuild the zyncAPP and get the same image id, as `reproduce/` does for
  the core. A zyncAPP whose source is hidden cannot be trusted, whatever its receipt says.
- **Its id is published** next to the core ids, with the core versions it reads.
- **Nothing commercial in the guest.** Payment, accounts and rate limits live outside the proofs.

## 6. Examples

| zyncAPP | inputs | uses | answer |
|---|---|---|---|
| **zyncPOO** (proof of ownership) | coins with inclusion paths; a challenge; a signature by each coin's key | `utxo_roots`, closed at H | "these coins, worth X sat, are unspent at H and the holder signed the challenge". A private form commits only the total |
| proof of payment | block header, its block-forest path, a Merkle branch to the transaction | `block_roots` | "transaction T is in this chain at height h" |
| supply | none | `utxo_value`, closed at H | "the UTXO set at H holds X sat" (compare with the issuance schedule) |
| snapshot check | a Core `dumptxoutset` | `utxo_roots`, `utxo_value` | "this snapshot is the UTXO set at H" (`utxo_snapshot_check.py` does it outside the zkVM today) |

## 7. Open

- `check_core_statement` as a crate shared by the verifier and every zyncAPP.
- Freshness: coins are "at H". Moving H forward without re-proving from genesis is SWIFTSYNC_PROOFS.md §7 (epochs),
  not yet decided.
- Signature checks inside a zyncAPP should reuse the core's libsecp and script code, not a new implementation.
