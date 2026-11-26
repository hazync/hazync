# The header zyncAPP on the engine (26.11.2026)

engine/zapps/headers (3aa4fbb9): the zAPP's own guest source (prover/zyncapps/methods/headers) with two anchored edits
(no `no_main`/`entry!`; `env::finalize()` after the commit) and the engine's platform crates. Built into a copy of
keys-m26 (its own image: exe 9ff4a306..., vm 53ca5965..., the same VM as the core image). Build box, CPU, `hdr.sh`.

- **Execute, headers 0..40,319 from genesis** (20 retarget epochs; input built by hdrtool.py in RISC Zero's serde word
  format, as the zAPP host's headers-exec writes it): 10.3 s, cost 6.29B, 112.9M instructions (~2,800 per header),
  journal 1,228 bytes. The decoded end state against the node's `getblockheader 40319`: **hash MATCH, chainwork
  MATCH, bits/time MATCH**; the 6 MMR peaks equal an MMR built independently from mmr.rs's spec over the file's hashes.
- **Prove, headers 0..4,031** (two epochs, CPU, the box shared with a regtest host build): 289.5 s; **`hzengine verify`:
  OK** (0.05 s), journal 1,228 bytes, sha256 d1db000b...
- Not compared byte for byte with RISC Zero's journal (no zAPP host build on the box); the reference used is Bitcoin
  Core's node, which is what the zAPP's own headers-exec checks against.
- Address space: the whole chain (975k headers, ~78 MB of input) does not fit the 2^26 keys in one run; runs chain
  (the zAPP's `--after`), or the zAPP uses keys with a larger address space (it is its own module and image).

## The WHOLE chain, 0..975,278, in one proof on one RTX PRO 6000 (26.11.2026)

Image on the production VM (2^29; keys-hdr29: exe a080482e..., vm 00e00077...), the headers file RISC Zero's MMR run used
(/root/mmr-v027/headers_0_975278.txt.gz, 975,279 lines).

- CPU execute (build box, hdrall.sh): 25.5 s, cost 152.1B, 2.73B instructions, journal 2,124 bytes sha256 20bb754b...;
  end state against the node's getblockheader 975278: **hash MATCH, chainwork MATCH, bits/time MATCH**, the 13 MMR peaks
  equal to an independent MMR of the file's hashes.
- **GPU proof** (pod l2buds6zmw5uvo, 1x RTX PRO 6000 Blackwell **Server** Edition, pod/run_one.sh): 176 segments,
  **prove 287.3 s** (job 297 s), GPU busy 58%, peak 24,065 MiB; **`hzengine verify`: OK**, journal sha256 20bb754b...,
  the same as the CPU run's.

| | RISC Zero (de54b94f, RESULT_gpu_whole_chain_mmr.txt) | engine |
|---|---|---|
| card | 1x RTX PRO 6000 Blackwell Workstation Edition | 1x RTX PRO 6000 Blackwell Server Edition |
| work | 4.66B cycles, 4,739 segments | 152.1B cost, 176 segments |
| proof | **5,969.5 s (99.5 min)** | **287.3 s (4.8 min)**: **20.8x** |

The two cards are the same chip in two editions (Workstation vs Server), not the same SKU; the comparison is close to
like for like but not exact.

## Same card as RISC Zero's run: RTX PRO 6000 Blackwell Workstation Edition (26.11.2026, re-run)

Pod guq4qpuihtefl7 (nvidia-smi: "NVIDIA RTX PRO 6000 Blackwell Workstation Edition, 97887 MiB, 600 W"), the same bundle.
- 176 segments, **prove 266.2 s** (job 276 s), GPU busy 56%, peak 24,621 MiB; `hzengine verify`: OK; journal sha256
  20bb754b..., identical to the Server Edition run and the CPU run.
- **MMR root**, bagged from the journal's 13 peaks (mmr.rs: right to left, bag = SHA-256(0x02 || p || acc)):
  bb22154abf3cf886ebcb512fc0901401fb9ae8362b9f40e6c7b09711a8ceb29a = RISC Zero's receipt's MMR root (MATCH).

| | RISC Zero (de54b94f) | engine (a080482e) |
|---|---|---|
| card | RTX PRO 6000 Blackwell Workstation Edition | RTX PRO 6000 Blackwell Workstation Edition |
| proof | 5,969.5 s | **266.2 s: 22.4x** |
| end hash / chainwork / MMR root | node's / node's / bb22154a | the same |

Pod cost: $1.03 (22 min: most of it a slow CUDA download, 4 MB/s, before the proof).
