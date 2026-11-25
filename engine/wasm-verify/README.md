# OpenVM's verifier in WebAssembly (FEASIBILITY Q11, 25.11.2026)

A probe, not a product: can a browser check an engine proof?

- `openvm-abi-stable.patch`: OpenVM v2.1.0 (05f3b73e) with `abi_stable` removed from openvm-circuit. It was a hard
  dependency used for one type (`RVec`, in the metered executor's memory context), and it pulls in `libloading`
  (native dynamic loading), which cannot build for wasm32. `RVec` becomes `Vec` there; nothing in the verifier uses it.
- `src/lib.rs`: `verify_proof(vk, proof)` = `openvm_verify_stark_host::verify_vm_stark_proof` (vk = bitcode of
  VmStarkVerifyingKey: agg.vk + baseline; proof = the codec encoding, zstd), via wasm-bindgen.
- Inputs from `hzengine wasm-inputs --dir <keys> --proof <proof> --out <dir>` (vk.bin, proof.zst, proof_bad.zst with
  one encoded byte flipped).

Build: apply the patch to a copy of OpenVM v2.1.0 at ../../../openvm-wasm, then
`RUSTFLAGS='--cfg getrandom_backend="wasm_js"' cargo build --release --target wasm32-unknown-unknown` and
`wasm-bindgen --target nodejs --out-dir pkg target/wasm32-unknown-unknown/release/wasmv.wasm` (wasm-bindgen-cli 0.2.130).

**Measured** (block 130,000's aggregate proof from the CPU segpar test, which `hzengine verify` also accepts; laptop,
Node 24): wasm 708,710 bytes; vk 96,425 bytes; proof 229,466 bytes zstd (273,107 encoded); **valid proof accepted in
24-45 ms, the flipped-byte copy rejected (0)**; native verifier on the same bytes 0.060 s (build box).

This checks the STARK against the key and baseline. A page also needs the image check (SHA-256(exe ‖ vm) == id) and
the journal check (public values == SHA-256(journal)), both cheap, from the proof's public values.
