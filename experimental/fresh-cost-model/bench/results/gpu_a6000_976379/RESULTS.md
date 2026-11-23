# GPU prove, block 976,379, RTX A6000 48 GB (23.11.2026)

One card, CUDA builds of all three VMs. Each proof is the un-aggregated
form (risc0 composite receipt, SP1 core proof, OpenVM app proof); all verified.
Pod ia48sq4c8a9n1h, $0.69/hr, up 13:05-16:11 UTC = 3.1 h, about $2.14.

| workload | risc0 3.1 | SP1 6.10 | OpenVM 2.1 |
|---|---|---|---|
| ECDSA x7,060 | 4,392.3 s | 399.2 s | 130.5 s |
| Schnorr x899 | 521.1 s | 43.2 s | (no BIP340 in OpenVM) |
| txid + merkle, 4,395 tx | 42.8 s | 9.9 s | 10.2 s |
| per ECDSA sig | 0.622 s | 0.0566 s | 0.0185 s |

risc0 and SP1 outputs match exactly (count + merkle root). OpenVM's
`verify app` passed; its outputs matched the other two VMs on the CPU executor.

Scaled to signatures only (3.58B ECDSA + 0.31B Schnorr, see data/aggregate.json),
$0.69/hr A6000, excluding sighash hashing, script, UTXO and aggregation:
risc0 ~669k GPU-h (~$462k), SP1 ~60k GPU-h (~$42k), OpenVM ECDSA-only ~18.4k GPU-h (~$12.7k).
