# hints zAPP: an engine proof (27.11.2026)

keys-z-hints's proof for H = 200, proved on CPU on the build box (engine/results/zapp_hints). Its child is keys-ch's
mainnet core proof of block 200, proved from genesis and verified in the guest. `hzengine wasm-inputs` wrote proof.hze
(HZE1: the proof in OpenVM's plain encoding, then the journal), vk.bin and vk.sha256; image_id = sha256(exe ‖ vm) of
keys-z-hints. mainnet-200.hints (sha256 a1d359dd...) and mainnet-200.undo come from the zAPP host's native
derivation for the same chain. The core image: keys-ch c7fe0eb8....
