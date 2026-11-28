# The engine release's real-GPU gate fixtures (PENDING: made from the frozen image)

scripts/release_gpu_gate.py proves these on rented cards with the release's exact hzengine CUDA build and key
tarballs, and verifies every proof with the release's verifiers (engine/RELEASE_TOOLING.md). Until `GATE` exists here
the gate refuses to start ("Nothing to gate"), which is the intended state before the Stage 4 freeze.

`GATE`, one entry per line:

```
# <label> <kind> <keys> <input> <height> <block hash> <anchor|-> <want>
b170 block core export_170.tgz 170 <hash> state_169.json DEMO
...  block core export_<h>.tgz <h> <hash> state_<h-1>.json DEMO      (a full, large block)
hdr  headers headers headers_0_<tip>.stream.gz <tip> <hash> - OK      (the whole header chain)
```

- `kind`: `block` = a .tgz of the product host's `engine-export` directory for that block, proved by
  `hzengine prove-block`; `headers` = the headers zAPP's input stream (gzip allowed), proved by `hzengine prove`.
- `keys`: the record (reproduce/engine-release.tsv) it proves under; the pod gets the staged `hazync-keys-<keys>.tar.gz`.
- `anchor`: the state the block's proof starts from (the gate proves a base step), or `-` from genesis.
- `want`: the verdict the staged verifier must give (`DEMO`: exit 2 from an anchor; `OK`: exit 0, the whole header
  chain with its tip compared).
- `b170` is required: the Windows run (release-gpu-gate-windows.ps1, record-windows) proves it.

`SHA256SUMS` lists every input and anchor; the gate checks them before renting anything, and the pod checks them again.
The header-chain stream (the whole chain: ~78 MB of input, engine/results/zapp_headers) is too large for git: it is to
be fetched by its sha256 from Hazync's storage, or built on the pod from the headers file. Decide when making them.
