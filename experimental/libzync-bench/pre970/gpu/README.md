# Mainnet 969,996..970,000 on 1x RTX PRO 6000 (2026-10-30, image `09a02eed`)

Evidence for proposal §10a. The image is the one after review round 1, before rounds 2-8.

- `verify.log`: ⚠ the path `/workspace/sb/signet.hints` is the **mainnet** utxohints.store file (`bitcoin.hints`,
  stop 970,000); the pod scripts were reused from the signet run and kept that file name. Its hash is the mainnet
  test vector `e215fe7e…`. The two FAIL lines are expected: the chain starts from a state taken from block
  969,995's file, not from genesis, so the ECMH total cannot close and `ss_hints` covers only five heights.
- `times.tsv`, `gpu_run.log`, `block_970000.rcpt`, `state_970000.json`: the run, the final receipt and its state.
