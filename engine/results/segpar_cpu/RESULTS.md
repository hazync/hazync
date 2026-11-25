# segpar end to end on CPU (25.11.2026)

Host 37a02971 (leaf workers reused for the aggregate's segments). Block 130,000 (export130000t: 1 chunk + trie +
aggregate), CPU engine, `CUDA_VISIBLE_DEVICES=0,1` (two pretend devices, so two leaf workers on "different GPUs"),
`--workers 2`, `HZ_SEG_MEM_GB=1` so the aggregate has many segments.

The aggregate's 19 segments were split 0..11 on a reused leaf worker and 11..19 in prove-block's own process, the app
proof assembled from both, then the SDK's aggregation, deferral and wrap steps: **the final proof VERIFIED**
(`hzengine verify`: OK). Chunk and trie journals IDENTICAL to RISC Zero's; the aggregate's RISC Zero's except its 32
image-id bytes. CPU times (1,287 s in all) say nothing about GPU speed; this is the correctness check that a proof
assembled from segments proved in two processes is a valid proof.
