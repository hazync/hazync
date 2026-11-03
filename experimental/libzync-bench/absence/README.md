# E1: proofs of absence (outpoint-keyed tree) — estimate first

⛔ EXPERIMENT. Nothing here is in the guest.

The coin log (COINLOG.md) is in birth order, so it proves what a coin IS, never that an outpoint is ABSENT
("X was never created", "X is not live"). That needs a second tree keyed by outpoint, updated every block: one
delete per spend, one insert per surviving output.

## Estimate (`estimate.py`, 11-03)

Compressed sparse Merkle tree, uniform keys, 180 M live coins (depth ~27), a block's updates applied as one batch,
old and new root recomputed over the touched paths; cycles per hash from the coin-log bench (100-525).

| block | updates | hashes | cycles (100 / 250 / 525 per hash) | vs whole block |
|---|---|---|---|---|
| 962,000 | 20,850 | ~616 k | 62 / 154 / 323 M | +14% to +74% of 439 M |

(`estimate_962000.out` also has 741,000.) For comparison the whole coin log costs **+2.1%** over 10 blocks.

## Measured hash cost (11-03, `zyncapp bench-hash`, execute mode)

| node hash | cycles per hash |
|---|---|
| one SHA-256 compression from a per-level IV (the coin log's `parent`) | **283** |
| SHA-256(0x01 ‖ L ‖ R), 65 bytes via `sha2` (the block forest's) | 1,105 |

With the coin-log hash, 962,000's ~616 k hashes are **~174 M cycles, about +40% of the block** (439 M), against +2.1%
for the coin log. The coin-log hash is the one to use if this is built.

## What it would unlock

- "Outpoint X does not exist / is not live" with a short proof. Most uses are already covered positively: a spent
  coin has a spent-by proof; a coin that never existed has no creating transaction to show.
- The valuable absence statements are about **scripts**, not outpoints ("these are ALL the coins of my keys", for
  proof of reserves). An outpoint-keyed tree does not give those; a script-keyed tree would, at a similar or higher cost.

## Plan

Build behind a guest feature after the rest of the roadmap, measure on 962,000 in execute mode, and decide with the
measured number. Script-keyed is the variant worth measuring first if absence is wanted for proof of reserves.

## Recommendation (11-03)

Do **not** put the keyed tree in the core: +40% on every block for a fact few zyncAPPs need. Build it as its own
recursive zyncAPP chain beside the core instead: each step verifies the core receipt for block h (assumption), opens
block h in the core's block forest (the hash binds the block's bytes), parses its transactions (no scripts: the core
already checked them) and updates the keyed tree. The core stays at +2.1%; only users who want absence proofs pay for
that chain. A script-keyed variant (all coins of a script, for proof of reserves) fits the same shape.
