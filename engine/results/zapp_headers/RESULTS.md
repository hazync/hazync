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
