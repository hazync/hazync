# Handover across OpenVM versions (design, 26.11.2026)

User decision (26.11.2026): build the handover now, so an OpenVM upgrade can never strand the proved chain.

## The problem

The engine's chain is a recursion: block k's aggregate verifies block k-1's proof in-guest (OpenVM deferral,
`env::verify`). That check is made by OpenVM's verify-stark deferral circuit against the aggregation verifying key of
the SAME VM. A minor OpenVM version changes the recursion circuits and therefore that key (VERSIONING.md: only patch
versions keep `MultiStarkVerifyingKey` compatibility). After an upgrade, the new image's deferral circuit cannot check
the old chain's last proof, and the chain would have to restart.

## The design: the previous version's verifier, as software, inside the new guest

OpenVM's host verifier (`openvm-verify-stark-host::verify_vm_stark_proof`) is plain Rust with no OS needs once
`abi_stable` is out (engine/wasm-verify: it builds for wasm32 and accepts real engine proofs). Compiled for the guest
(riscv32, no_std + alloc), it runs inside a zkVM like any other code. So:

1. **Pin.** Each released engine image records its predecessor: the previous version's aggregation verifying key and
   baseline (the `VmStarkVerifyingKey` the wasm verifier takes), its image id (SHA-256(exe || vm)), and the handover
   height H. As RISC Zero's handover_pin.rs does, the pin is part of the new image's source, so it is in its image id.
2. **Handover proof.** At H, the new image's aggregate for block H+1 takes the old chain's proof of block H as plain
   input bytes (not a deferral), runs the PREVIOUS version's verifier on it in software, and checks what the verified
   proof commits: its image id against the pin, its public values against SHA-256(old journal), the old journal
   decoding as ChainState at H. Then it continues the chain from that state, exactly as a recursion step would.
   The check runs ONCE per upgrade, so its cost (a software STARK verification) does not matter for throughput.
3. **Lineage.** The new ChainState carries the lineage (the list of image ids since genesis), as the RISC Zero chain's
   `lineage` field does, so a verifier knows which images it rests on and their order.
4. **Verifiers.** A verifier of the new chain checks the new image's proof only: the handover is inside it. To check
   the old chain itself, the old release's verifier is still the tool.

What this needs from OpenVM, none of it a fork: the verifier crate building for the guest target (the wasm patch's
`abi_stable` removal, upstream or carried), and its cost inside the zkVM, to be measured.

## Alternatives, and why not

- **SNARK wrap (OpenVM's root prover to Halo2/KZG):** cheaper to verify in-guest, but a trusted setup and a second
  proof system on the soundness path; kept as a fallback if the software verifier turns out too costly.
- **Keep the old version's prover forever:** no handover, but every upgrade then forks the chain.

## Plan

1. Measure: OpenVM v2.1.0's verifier compiled into an engine guest mode, verifying a real engine proof (today the "old"
   and "new" versions are the same, which tests the mechanism and gives the cost).
2. The pin and the handover mode in the guest (aggregate input carries the old proof; the checks above), behind a
   build-time pin so an image without a predecessor cannot take that path.
3. A rehearsal: two images from different OpenVM tags (2.1.0 -> a 2.1.x or the next minor), a short chain on the old,
   handed over to the new, verified end to end; negative controls (a wrong pin, a tampered old proof, a journal that
   is not the old proof's).
