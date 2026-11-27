# The whole header chain, 0..975,278, on the engine (27.11.2026)

`proof.hze`: one engine proof of headers 0..975,278 (image 25b32ac5..., keys-hdr29: exe a080482e..., vm 00e00077...,
the 2^29 VM), proved on one RTX A6000 (RunPod pod gbk6ztfwyododf, `engine/scripts/gpu_block.sh` with RUN=run_one.sh
HARVEST_PROOFS=1, bundle_hdr.tgz as on 26.11): prove 722.0 s, job 743 s, GPU busy 65%, peak 23,856 MiB; `hzengine
verify` OK, journal 2,124 bytes sha256 20bb754b... (the same as the 26.11 runs), packed with `hzengine pack` (323,287
bytes). `vk.bin` is that image's key (pin in vk.sha256), `image_id` its id.

Checked: `zapp-headers --image $(cat image_id) --vk vk.bin --vk-sha256 "$(cat vk.sha256)" --expect 975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3`
gives SWIFTSYNC_VERIFY_OK (chainwork 0x15731592c05251d96d07f062f >= Core's minimum; MMR root bb22154a..., RISC Zero's
receipt's root); another tip hash FAIL. The tip hash is hazync-coord's node's `getblockhash 975278`.
