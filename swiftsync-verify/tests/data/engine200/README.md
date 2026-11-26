# engine200: the engine chain's proof of mainnet 1..200 and facts on it (engine/VERIFIER.md)

- `proof.hze`, `vk.bin`, `vk.sha256`, `image_id`: `hzengine wasm-inputs --dir keys-ch --proof core200/block_200.ovm`
  (keys-ch: node format 2 + C heap, 2^29), the image id SHA-256(exe || vm) of keys-ch/image.json.
- `history.bin`: the chain run's (engine/scripts/pod/run_chain.sh).
- `coin_*.json`: `zync-path coin history.bin 200 200 <pos>` (block 1's coinbase with its preimage added by hand from
  block 1's coinbase; block 9's coinbase spent in block 170).
- `trie_*.json`: `zync-path trie coin|balance|history zapps/statement/vectors/engine_200.trie <script> [<txid> <vout>]`
  (that trie from native prep: `host libzync-prep 1 200 <blocks> genesis <out>`), each with its claimed `expect`; the
  coin's `origin` from the journey below.
- `journey_hal_170.json`: `zync-path journey history.bin 200 170 --bitcoin-cli fake-cli-170.sh` (a stand-in node that
  serves block 170 and block 169's median time past from the repo's mainnet block files: nothing else).
