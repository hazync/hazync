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

The 3.0.5 receipts are kept as test vectors only: 3.1.0 fixed soundness bugs in 3.0.x, so no image may hand over from
one (docs/HANDOVER.md §1). The 3.1.0 row is the case that matters next: the published images run 3.1.0, so the image
that moves the board to a later RISC Zero pins its predecessor through `xver::v3_1_0`.
