# zeroconf zAPP: an engine proof with real children (27.11.2026)

From engine/zapps/zeroconf/real_proofs.sh (build box, CPU). The production zeroconf image keys-z-zeroconf2
(e1e5aaf5...) proved that T (T.json, the fixture's: Core v31.1 accepts it at 110) is valid for block 111 on the
regtest harness's tip 110 (2d5cc9c8...). Its children were verified in the guest:
- the core proof of block 110: keys-rtch (2d740159...), a base step on native prep's state at 109
  (anchor_state_109.json);
- the speculative chunk for 111.

`hzengine wasm-inputs` wrote proof.hze (HZE1), vk.bin and vk.sha256.
