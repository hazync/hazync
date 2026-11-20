# libzync test data

Inputs for the libzync soundness jobs in CI (`.github/workflows/adversarial.yml`, `libzync-*`; drivers in
`scripts/libzync/ci/`). Block files are the host's JSON format (`prover/fetch_block_rpc.py`), fetched from a mainnet /
signet archive node; the guest checks every byte it uses (header hash, merkle root, PoW), so a wrong file fails a test,
it cannot make one pass.

| path | what |
|---|---|
| `mainnet/block_1..200.json`, `mainnet/hashes.txt` | mainnet blocks 1..200 and the block hashes 0..200 (genesis first), for the run from genesis checked against `scripts/libzync/coinlog_ref.py` and `block_forest_ref.py`, and the leaf and join tests on it |
| `signet/block_1..100.json`, `signet/hashes.txt` | signet blocks 1..100 and hashes 0..2599 |
| `gpu-gate/` | the release's real-GPU gate (`scripts/release_gpu_gate.py`, `docs/BUILD_AND_RELEASE.md` "The real-GPU gate"): `GATE` (heights, block hashes, files), the real-state leaf bundles of blocks 170 and 200,000 as `libzync-prep` cut them on `hazync-proof` under image `46666b6f`, prep's `state_<h-1>.json` (the verifier's anchor) and `state_<h>.json`, and `SHA256SUMS` |
| `edge/block_<h>.json` | edge blocks: the first retarget (2016), the BIP30 repeats (91842, 91880), 185956, an activation boundary (227931), 363725, the largest legacy transaction (364292), 388381; 419328 and 481824 are read from `../boundaries/` |
