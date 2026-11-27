# utreexo zAPP: engine proofs (27.11.2026)

Proved on the engine on CPU (build box, engine/results/zapp_utreexo: zt2.sh, zt3.sh). Both have keys-ch's
mainnet core proof of block 200 (proved from genesis) as a child, verified in the guest. `hzengine wasm-inputs` wrote
each file (HZE1: the proof in OpenVM's plain encoding, then the journal) and its key file.

| file | image (sha256(exe ‖ vm)) | key pin (sha256 of the .vk) |
|---|---|---|
| base100.hze: the base, roots after block 100, core tip 200 | keys-z-utreexo 340f6675... | a966e0eb... |
| step150.hze: a tip step 100 -> 150 on that base | keys-z-utreexo-tip 6ecf6f43... | 788f9ef4... |

The core image: keys-ch c7fe0eb8.... ids.txt lists all of them in full.
