# Hazync's C++ consensus path on OpenVM 2.1 — CPU executor, block 976,379 (23.11.2026)

Guest `ovm-core/`: Bitcoin Core v28.0 + patches 0001 0002 0009 0010 libzync/0101 (Hazync's `scripts/libzync-base.sh`,
main 261f0a59), Hazync's `verify_input.cpp` / `core_tx_rules.cpp` / `core_block_rules.cpp` / `cshims.c`, product Core
levers (SHA_FASTPATH, SHA_D64_ACCEL, SHA_STREAM, HASHWRITER_BUF, LAZY_TXID, H160_MEMO, LEGACY_SIGHASH, D64_STREAM,
RAW_TXID, PUBKEY_MEMO, LEAF_ONEBUF, TXDATA_LAZY, AGG_LITE, TX_ONCE), libstdc++/newlib from the same riscv32 gcc 13.2.
risc0's `sys_sha_compress` (which the patches call) implemented on OpenVM's SHA-256 extension. libsecp v0.5.1 +
`ovm-secp/apply_hook.py`: batch hooks (ECDSA kind 2, BIP340 kind 1, BIP341 tweak kinds 3/4), liftx sqrt, and a
DIRECT accelerated check (u1·G + u2·Q) for any call without a valid hint. Driver: `verify_inputs_batch_once` over
every input of every non-coinbase transaction, flags 134677, `hz_cache_reset` per transaction. Hints: a COLLECT pass
logs every deferred check, `hints/hints2` computes R natively.

| run | result | instructions | OpenVM cost |
|---|---|---|---|
| COLLECT (libsecp verifies every signature itself, liftx on) | 8,130 inputs / 4,394 txs ACCEPTED | 15.56B | 598.5B |
| **BATCH (product path)** | 8,130 / 4,394 ACCEPTED; deferred 8,264, keys 3,669, direct 72 | 542M | **21.8B** |
| NO_SCRIPTS (same, VerifyScript skipped: parse, wtxid, seqs) | — | 72.7M | 3.1B |

Breakdown (BATCH): non-script tx work ~3.1B; signatures ~8.7B (ovm-secp mode 5, 7,959 sigs); Core interpreter +
sighash + txdata ~10B by difference. Estimated proving (k256 ratio 8.77B -> 130.5 s, NOT validated for this guest):
~325 s app + ~7% wrap on one A6000, vs Hazync product chunks 1,172 s on the same card (run 2).

Bugs found on the way: (1) without `__libc_init_array()` Core's static tagged-hash midstates are zero -> every taproot
spend fails (Hazync's guest calls it; documented). (2) `HAZYNC_LEGACY_SIGHASH` WITHOUT `HAZYNC_AGG_LITE`: the legacy
cache keys only on vin/vout data pointers + counts + version + locktime -> consecutive same-shape transactions reuse a
stale preimage (389 valid P2PKH inputs rejected here). The product sets both levers; nothing enforces it.
