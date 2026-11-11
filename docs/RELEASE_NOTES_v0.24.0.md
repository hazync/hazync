<!-- DRAFT for discussion with the operator before any release is cut. Lines marked [DISCUSS] are open. -->

v0.24.0 is libzync on **RISC Zero 3.1.0**. The guest source is the same as in v0.23.0. What changed is
the proof system beneath it: 3.1.0 fixes soundness bugs in RISC Zero, and v0.23.0's proofs were made with
the version those bugs are in. **Use v0.24.0. v0.23.0 is withdrawn and must not be deployed.**

**Why.** RISC Zero 3.1.0 fixes these bugs in the zkVM's proof system, and has yanked 3.0.x:
- an rv32im trace could hold two executions;
- `sys_read` words were not range-checked;
- the recursion circuit's `BIT_AND_ELEM` decompositions were not unique;
- a Groth16 claim could be substituted;
- the Poseidon S-box.

None of these bugs is in libzync's own code, but a proof is only as sound as the system that checks it.
Verifiers built on 3.1.0 refuse every 3.0.x receipt, because the verifier parameters differ.

**The images change.** The guest source is unchanged, but the image id depends on the toolchain, so every
id moved:

```
mainnet  8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4
signet   d4210dae455340743cca097f2b422eee9846c9c02af0dae31d1268fadef4f387
headers  2db34e12cc993dd1edb003f00f85c93d02e0e61208589b57fc5c969d39b3e632
```

`reproduce/libzync.Dockerfile` rebuilds them bit for bit (`reproduce/LIBZYNC_IDS`).

This release's verifier refuses proofs by v0.23.0's image `863c9ce3…` (exit `1`). Those proofs must be
re-made under `8888d59c`.

## What changed in what you download

The asset names and what each one does are the same as in v0.23.0. Every binary is rebuilt on 3.1.0:
- the host and the CUDA host, for Linux and for Windows;
- the CLI verifier and the browser verifier (`hazync-verify.wasm` sha256
  `446fdaeb5a2937d6f442e16058969d83af8c6ddebdbaba103d8d934787ea107f`);
- the proof-party tools.

The verifier's three answers are unchanged: `0` OK, `2` DEMO (valid, not yet at minimum work), `1` refused.

**A provers' note:** run-workers.sh checks the image id only at startup. Any worker loop started on
v0.23.0 must be restarted on v0.24.0, or it will keep producing proofs the board refuses.

## Measured on 3.1.0

One NVIDIA RTX PRO 6000 per run, 2026-11-11 (`experimental/libzync-bench/RESULT_risc0_3_1_gpu.txt`).

**Mainnet genesis to block 200**, proved by the published image:
- 3,074 s;
- the result is a 227 KB receipt;
- the verifier answers `DEMO`;
- the same receipt is refused when checked as signet, when checked against the old image, and with one
  bit flipped.

**The whole header chain, genesis to block 975,278**, in one proof:
- 59.4 min, against 57.2 min on 3.0.5;
- verifies `OK` against a full node's tip hash.

These four cases are now permanent test vectors for the CLI, the C library and the WASM verifier.

## The board

[DISCUSS: the board's prep state was built under 863c9ce3. It carries over by being re-stamped with the new
image id, not re-prepared. Leaf bundles proved under 863c9ce3 must be re-proved. Give the frontier
figures at the time of release.]

## Not yet

These are unchanged from v0.23.0:
- **No independent audit.**
- **No proof of the whole chain yet.**
- **Following the tip with libzync** is not running yet.

[DISCUSS: tone, and whether to repeat the "v0.23.0 withdrawn" line at the top of the v0.23.0 release page (it is already there).]
