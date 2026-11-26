# The hints zyncAPP end to end on the engine, on an engine core chain from genesis (26.11.2026)

**The core chain.** Blocks 1..200 of mainnet proved from genesis through the product host on the engine (pod
c91yr9nqi954dk, 1x RTX A6000, pod/run_chain.sh): image keys-ch (node format 2, C-heap recycling, 2^29), host built in the
glibc-2.35 container at a703c910. **200 blocks in 2,866 s (14-20 s each: chunk(s) + trie + aggregate verifying the
previous block)**, every block verified by the next; the last proof checked alone: `hzengine verify` OK. State at 200:
profile 0xf1 (libzync, coin log, counting trie, live-coin hash, node format 2). Pod ~$0.65.

**The hints zAPP** (engine/zapps/assemble_guest.sh hints: its own source + the shared coin log edit; image keys-z-hints):
- input: the zAPP host's `hints-prove ... --stream` (the RISC Zero ExecutorEnv's values as words), H = 200, from the
  core state; the host's native derivation: hints file sha256 a1d359dd..., counted 209, unspent 204, 10,000 BTC;
- `hzengine execute` with the core's block-200 proof as the child (`HZ_CHILD_KEYS` = the core image's keys; the guest's
  `env::verify(core_id, journal)` is OpenVM deferral): rc 0, cost 0.11B;
- `hzengine prove` (CPU, build box): 237.1 s; **`hzengine verify`: OK**; journal 988 bytes sha256 a32f5be1...;
- the statement carries the native derivation's hints sha256, block 200's hash and the height.

So a zyncAPP can verify an engine core proof inside its own proof and commit a statement about it, as on RISC Zero.
