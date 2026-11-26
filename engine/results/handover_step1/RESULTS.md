# Handover step 1: OpenVM's verifier as software inside an engine guest (26.11.2026)

engine/handover (7b16bf5c): verifier-guest = openvm-verify-stark-host's `verify_vm_stark_proof_decoded` compiled for
the guest, with OpenVM v2.1.0 + engine/wasm-verify/openvm-abi-stable.patch + openvm-verify-nozstd.patch (the
compressed encoding optional) and stark-backend v2.1.0 + stark-sdk-nologging.patch (the logging stack optional: a
guest has no clock), every openvm / stark-backend crate from one patched copy. Image exe a5382774, VM 00e00077 (2^29).
Build box, CPU, `hzengine execute` (hv.sh).

Input: a REAL engine proof (block 130,000's aggregate, keys-hz24; the uncompressed encoding, 273,107 B) and its
verifying key (agg.vk + baseline, 96,425 B).

| case | verified | image id derived in-guest | public values | cost | instructions |
|---|---|---|---|---|---|
| the proof | **1** | c45fb756... = keys-hz24's image id | 9ac1acb3... = SHA-256(block journal) | **15.55B** | 422.7M |
| one proof byte flipped (index 100,000) | **0** | c45fb756... | (decoded, not verified) | 9.22B | 249.2M |

So the mechanism HANDOVER.md rests on holds: a guest can check a proof of another OpenVM image (here the same OpenVM
version; an upgrade puts the previous version's verifier there instead), derive what it proves and refuse a tampered
one. The cost is paid once per upgrade: 15.6B is about 60% of block 976,379's whole proving work (25.8B).
Next: the pin + the handover mode in the aggregate, then a rehearsal across two OpenVM tags (HANDOVER.md Plan 2-3).
