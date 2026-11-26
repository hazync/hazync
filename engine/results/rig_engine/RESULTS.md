# The core-rules rig on the engine, first run (26.11.2026, one A40)

experimental/libzync-bench/core_rules/rig.py: Bitcoin Core 31.1 on regtest and the regtest engine image (keys-rt,
2^26) given the same crafted blocks on a 232-block base chain; Core's submitblock answer is the reference. Real proofs
on the GPU (the base chain: 232 blocks proved through libzync-prove-range on the engine, ~16-22 s per block).
pod/run_rig.sh, pod 728rss4avwkcsa, 3h20 in all with Q7 ($2.46).

**55 of 96 cases ran; all 55 AGREE** once read as rig.py reads them (rig_reclassify.py, reclassified.txt):
the valid blocks accepted, every invalid one refused by the guest with the reason the rig's own table (OURS /
reason_ok, taken verbatim from rig.py) requires for Core's answer: coinbase rules, merkle and witness commitments,
duplicate / empty / negative / oversized outputs, premature coinbase spends, double spends, locktime / BIP68 (height
and time), MTP, version, proof of work, nBits, witness nonce, sigops and weight limits, missing and spent coins.

How rig.py's raw labels became these, and why (no verdict changed, only where the guest's text is read from):
- 36 cases were labelled HOST-FAIL because rig.py looks for the guest's verdict in the HOST's output, where RISC
  Zero's executor prints guest output. On the engine the guest's output is in the engine's work dir, and the host's
  panic on a failed aggregate carried none of it. rig_reclassify.py adds each case's guest logs to the text and
  applies rig.py's in_guest test and reason_ok unchanged, with one mapping: RISC Zero's host reports a guest panic as
  "Guest panicked", on the engine it is the guest's own "panicked at" line in its log. Fixed in the host since
  (engine_backend: a failed execute now panics with the end of the guest's log).
- missing_coin: rig.py defers it unless the image has a coin log (RIG_COINLOG=1); the engine image has one, so it
  is judged like the rest: refused at the block, AGREE.
- The run stopped at split_tx (case 56): rig.py asserts the block spans 2 chunks, and on the engine the host's
  default cost target overrode the rig's HAZYNC_CHUNKS=2 (1 chunk). Fixed (f4a7e229: an explicit HAZYNC_CHUNKS alone
  is a fixed count). The 41 cases from split_tx on (signatures, taproot, multisig, CLTV/CSV, P2SH, BIP30 repeats,
  ...) did not run: next run, with both host fixes built.
