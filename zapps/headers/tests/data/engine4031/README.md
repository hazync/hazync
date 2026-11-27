# engine4031: the engine's proof of mainnet headers 0..4,031

- `proof.hze`, `vk.bin`, `vk.sha256`, `image_id`: keys-hdr's proof of headers 0..4,031 (engine/results/zapp_headers,
  proved on CPU 26.11.2026), written by `hzengine wasm-inputs`.
- `mmr_path_<h>_tip4031.txt`: block h's MMR path to the tip 4,031, one 64-hex hash per line, lowest first, as
  `zyncapp headers-mmr-path` prints it. 200 was made by that tool; 170 and 37 (27.11.2026, for wasm/test-headers.mjs)
  by `mmrpath.py` (zapp_headers::mmr::path in Python) from the box's `h0_4031.txt` (engine/results/zapp_headers/hdr.sh
  fetched it from the node), after it reproduced the 200 file byte for byte.
