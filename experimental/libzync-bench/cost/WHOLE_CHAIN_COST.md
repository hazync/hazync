# What a whole-chain libzync proof would cost (measured survey, 2026-10-30)

**Method:**
- **Sample:** 19 real mainnet blocks, every 50k from 100,000 to 970,000.
- **Execution:** each block executed on the laptop with today's libzync guests, scripts ON (round 16) and scripts OFF
  (round 17, `HAZYNC_NO_SCRIPTS`), with real SwiftSync hint bits from the published `utxohints.store` mainnet file.
  Data: `survey.out`.
- **Scaling:** each sample stands for its era, between the midpoints. Era cost = blocks × fixed per-block base + era
  transactions (cumulative counts from `getchaintxstats`, in `txcounts.txt`) × the sample's cost per transaction.
  Script: `estimate.py`.
- **Cycles → GPU seconds:** today's measured rates (PRO 6000 ~1.2 s/Mcycle, A40 ~2.7), plus ~8.5 s (PRO 6000) /
  ~17 s (A40) of fixed overhead per proof process, two processes per block today.

## Result: `estimate.out`

| | total work | A40 (cheapest) | PRO 6000 |
|---|---|---|---|
| mainnet 1..970,000, scripts ON | 257.7 T cycles | ~194–202k card-h, **~$114–119k** | ~$214–225k |
| mainnet 1..970,000, scripts OFF | 82.5 T cycles | ~62–71k card-h, **~$37–42k** | ~$68–80k |
| signet 1..320,000, scripts ON | 3.0 T | $1.3k batched – $3.1k per-block | $2.5–6.3k |
| signet 1..320,000, scripts OFF | 1.55 T | **$0.7k batched** – $2.5k per-block | $1.3–5.0k |

## What drives it

- **Mid-history is the expensive part.** The 350k–700k samples cost 150k–450k cycles per transaction, against ~70k at
  962,000 and 970,000. Legacy multi-input transactions and consolidations batch less well.
- **Scripts OFF cuts total work about 3×.** That is enough for a hints-file proof, which is about coin accounting. A
  scripts-off proof says nothing about script validity: it is the assumevalid model.
- **Per-block fixed overhead dominates tiny-block chains** (signet, early mainnet). "Batched" means many blocks in one
  proof session. It is **not built**, and for signet it is worth ~4×.
- **Signet is not ready.** The guest is mainnet-only: genesis, fork heights and difficulty, and no signet block-solution
  check. Supporting it is real guest work, not configuration.

## Caveats

- One sample per era.
- Cycles → seconds is extrapolated from ~22 M-cycle chunks.
- No volume pricing.
- Treat everything here as ±50%.
