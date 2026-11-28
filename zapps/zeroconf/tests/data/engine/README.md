# zeroconf zAPP: an engine proof with real children (27.11.2026)

From engine/zapps/zeroconf/real_proofs.sh (build box, CPU). The production zeroconf image keys-z-zeroconf3
(1ee5f2d7..., the guest with its own kind 0x5A5A0001) proved that T (T.json, the fixture's: Core v31.1 accepts it at 110) is valid for block 111 on the
regtest harness's tip 110 (2d5cc9c8...). Its children were verified in the guest:
- the core proof of block 110: keys-rtch (2d740159...), a base step on native prep's state at 109
  (anchor_state_109.json);
- the speculative chunk for 111.

`hzengine pack` wrote proof.hze (HZE1) and vk.bin, and printed image_id and vk.sha256; core_image_id is keys-rtch's.
re-proved 27.11.2026 with the new kind (engine/zapps/zeroconf/real_proofs.sh, real3/: core 354 s, chunk 126 s, zeroconf
155 s).

**28.11.2026:** this proof is of image 1ee5f2d7, before the guest pinned the genesis anchor (commit 977439f0, image
132969d8...). Its core child is a base step at 109, which the new guest refuses ("not anchored at the network's
genesis"; engine/zapps/zeroconf/fixture/README.md). It stays as the reader's fixture: the reader's checks are the
same for both images. A proof of the new image needs the regtest core chain proved from genesis.
