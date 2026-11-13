# Cross-version handover: the old-circuit check inside the zkVM

`prover/methods/guest/src/xver/` lets an image hand over from a predecessor proved under an OLDER RISC Zero
(docs/HANDOVER.md §3). This harness runs that module, unchanged (`#[path]`), inside a RISC Zero 3.1.0 guest, on real
receipts, in the executor (CPU, no proving):

```
CARGO_TARGET_DIR=<somewhere on a big disk> cargo build --release -p xver-host
xver-host <succinct receipt> [<image hex>]
```

For each receipt: the valid case must verify, and four negative controls must make the guest panic: one seal word
changed, the wrong predecessor image, the wrong journal, and the other vendored RISC Zero version. Exit 0 only if all
five behave (`XVER-GUEST-OK`). `host xver-selftest <receipt>` is the same check natively, plus the verifier-parameter
digests recomputed from the vendored circuits.

## Results (2026-11-13, r0vm 3.1.0 executor, laptop CPU)

| receipt | RISC Zero | image | verify cycles | segments (po2 20) | negatives |
|---|---|---|---|---|---|
| `pre970/out/block_970000.rcpt` (227,034 B) | 3.0.5 | `09a02eed…` | 22,705,898 | 25 | 4/4 refused |
| `pre970/out/block_969996.rcpt` (226,258 B) | 3.0.5 | `09a02eed…` | 22,705,659 | 25 | 4/4 refused |
| the live board's spine [1..1466] (229,458 B) | 3.1.0 | `8888d59c…` | 22,712,731 | 25 | 4/4 refused |

## End to end in the hazync guest (2026-11-13, dev mode, laptop CPU)

A TEST mainnet image X (laptop build, id `534d79d6…`, not published): this branch's source with `xver-v3_1_0` on and
`8888d59c` pinned at block 1467 with `verifier: xver::v3_1_0::PARAMS`. Inputs, copied read-only from the live board:
the spine [1..1466] (`8888d59c`, 229,458 B), `leaf_1467.bin`, and the board's `state_1466.json` / `state_1467.json`.

- `host_X libzync-handover spine_1_1466.bin leaf_1467.bin out`: block 1467 ran as the handover step. The aggregate,
  EXECUTED (dev mode skips only the proving), checked the real 3.1.0 seal through the vendored circuit:
  `LZ_STATS aggregate h=1467 segments=26 total_cycles=27262976 user_cycles=24283751`.
- `host_X verify-any out 1 --check-end`: `RANGE-OK lo=1 hi=1467 … image=534d79d6… lineage=8f840aef… anchored=1 end=board`
  (the board's own state at 1467, re-stamped as X's).
- `host_X verify-any spine_1_1466.bin 1`: the predecessor's real spine is accepted as `image=8888d59c…`, through the
  vendored circuit natively.
- Negative: the same spine with one byte of its seal flipped is refused before anything runs (`xver: predecessor seal
  does not verify`).

Not done: a real (GPU) proof of such a handover. Dev mode proves nothing, so the aggregate's recursion over the real
seal check is untested until the next GPU batch.

The 3.0.5 receipts are kept as test vectors only: 3.1.0 fixed soundness bugs in 3.0.x, so no image may hand over from
one (docs/HANDOVER.md §1). The 3.1.0 row is the case that matters next: the published images run 3.1.0, so the image
that moves the board to a later RISC Zero pins its predecessor through `xver::v3_1_0`.
