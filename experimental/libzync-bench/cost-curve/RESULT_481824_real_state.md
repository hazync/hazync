# Block 481,824 on real state: the first (A) row above 400,000

This is open item §8.1 of [`RESULT.md`](RESULT.md) for its first height. It is the same measurement as the (A) rows of
[`README.md`](README.md), run on 22.11.2026 on `hazync-coord` with the same scripts (`run_a.sh` and `run_b.sh`, unchanged
from this directory at `fee4318a`) and the same published v0.27.0 host (`4c79a0f1…`, image `46666b6f`). The leaf
bundle is `leaf_481824.bin` from the current prep run on `hazync-proof` (the one started on 16.11.2026 under the fixed
host), copied read-only. Execute only (dev mode), segments at po2 21, the shipped two chunks. Nothing was proved.

## Result

| part | user cycles | padded cycles | segments |
|---|---|---|---|
| chunk 0, inputs [0..2466) | 163,632,184 | 195,035,136 | 93 |
| chunk 1, inputs [2466..5192) | 141,306,083 | 169,869,312 | 81 |
| trie | 78,542,423 | 93,323,264 | 45 |
| aggregate | 72,559,358 | 86,245,376 | 42 |
| **block** | **456,040,048** | **544,473,088** | **261** |

Wall time 117.6 s, peak RSS 1.16 GB, load average 47 at the start (the box is shared). `STATE_MATCH`: the committed
state equals prep's `state_481824.json` in all 23 fields but `anchor_id`, which differs by construction (README).
At R4 (`RESULT.md` §6) these padded cycles are about 565 s of card time on one RTX PRO 6000 (ESTIMATED, not proved).

## Against the modelled state (B)

`run_b.sh` on the same block, with the trie and the aggregate on the MODELLED state, in §3's form (A against B, user
cycles, millions):

| block | trie A / B (M) | aggregate A / B (M) | block total A / B (M) |
|---|---|---|---|
| 481,824 | 78.54 / 77.68 (+1.1%) | 72.56 / 72.24 (+0.4%) | 456.04 / 454.86 (+0.26%) |

The chunks are identical in A and B (163,632,184 and 141,306,083: a chunk does not depend on the state). At 481,824
the model understates the trie by 1.1%, the aggregate by 0.4% and the block by 0.26%, less than at any (A) height of
§3. Whether the trie error grows towards the tip is still not measured above this height.

## Raw outputs (`raw/real_481824/`)

| file | what |
|---|---|
| `leaf_481824.out` | (A) host output: `LZ_STATS`, `TIMED`, `STATE_MATCH` |
| `times_481824.tsv` | (A) the host's per-phase times |
| `state_481824.json` | (A) the committed state decoded from the aggregate's journal |
| `hints_481824.out`, `agg_481824.out` | (B) `libzync-hints` and `agg-exec-fake` output |
| `inputs.sha256` | the leaf bundle, prep's two states, the block file and the host (paths as on `hazync-coord`) |
