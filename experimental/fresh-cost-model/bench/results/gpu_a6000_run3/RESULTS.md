# Run 3: Hazync's Core C++ path on OpenVM, PROVED on one RTX A6000 (23.11.2026)

Pod jxa6047aizbxf8, $0.69/hr, 22:00:49-22:18:42 UTC = 18 min, about $0.21.
Executable `ovm-core-batch.vmexe` sha256 99b285ef...df1ada9 (rebuilt on the build box and re-metered first: 21,792,618,497
cost, output identical to results/cpu_core_ovm). Block 976,379: every input of every non-coinbase tx (8,130 / 4,394)
through Hazync's verify_inputs_batch_once with the product Core levers; libsecp v0.5.1 + batch/liftx hooks.

| proof | secs | verify | size |
|---|---|---|---|
| app proof (comparable to Hazync's chunk receipts) | **128.4** | Proof verified successfully | |
| STARK, ONE proof (wrapped) | **138.6** | Proof verified successfully | 630,764 B |

GPU utilisation mean 64%. Cost->time: 21.8B cost in 128.4 s = 5.9 ns/cost (k256 run was 14.9 ns/cost; my estimate of
~325 s from that ratio was 2.5x pessimistic).

Same card, same block, Hazync v0.26.0 product (run 2): chunks 1,172 s + aggregate 486 s = 1,658 s.
=> script/signature side: **1,172 s -> 128 s (9.1x)**. The aggregate (coin log, trie, block rules) is NOT ported yet.
