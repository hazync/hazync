# R2-5: end-to-end false statement across the seams (reviewer report, 27.11.2026, as given)

Could not make a false statement verify end to end: every seam pins correctly or is covered at the next layer. No
exploitable finding; hardening only. Read only.

1. LOW (hygiene; high that they are unused in shipped paths). Two public APIs still trust an unpinned key:
   engine/verify-core/src/lib.rs:116 verify_stark_bytes(vk, proof) (no pin, no image id, no journal check; returns 1;
   no caller; round 1 removed the same from wasm-verify but left it in the shared crate; forged mvk + forged proof = 1);
   swiftsync-verify/src/lib.rs:147 Image::with_engine_key(bytes) pins a key to its own SHA-256 (every shipped caller
   checks the caller's pin first: CLI main.rs:141 read_pinned_key, FFI swiftsync-verify-ffi/src/lib.rs:108-113, wasm
   swiftsync-verify-wasm/src/lib.rs:158-163, zapps/corecheck/src/lib.rs:101 with_key); with_key(.., None) unpinned, tests
   only. A library integrator using them on a key that came with the proof gets the forged-key acceptance back.
   Suggestion: #[cfg(test)] / private, or take the pin.
2. LOW (informational; high). The zeroconf guest's anchor check compares a value with itself
   (prover/zyncapps/methods/zeroconf/src/main.rs:95 passes &j.anchor_id as genesis_anchor). Not exploitable (the
   statement commits anchor_id; the reader checks it vs the pinned genesis, zapps/zeroconf/src/lib.rs:125-129), but the
   zAPP's soundness then rests entirely on the host check.
3. NOTE. Soundness depends on OpenVM's deferral circuit fixing the children's recursion key: env::verify
   (engine/platform/risc0-zkvm/src/lib.rs:156-177) checks a child only by sha256(app_exe_commit || app_vm_commit), which
   covers the app, leaf and internal-for-leaf keys, not the internal-recursive key (mvk). OpenVM's
   guest-libs/verify-stark/circuit/src/lib.rs carries a constant expected_internal_recursive_vk_commit in the parent's
   app key, as VERIFIER.md claims; the circuit itself not audited. If it were not enforced, every join, aggregate,
   zAPP-on-core and SAME_ZKVM handover would accept children under a forged key. (R2-1 point 3 covers this.)

Seams tried and sound: proof framing (unpack refuses a 0 or overlong proof length; journal = bytes after the proof,
bound by public values = SHA-256(journal), one byte per element, > 255 refused); pins in every reader (core and zAPP CLIs,
C ABIs, wasm frames: swiftsync-verify, assume, deposit core + headers keys, headers, hints, liveness, statement,
utreexo, zeroconf; a missing key fails closed; SSPF bundles cannot avoid the key check: check_bundle calls verify, the
header fields must equal the verified journal); self_id (a prover input, but the reader requires cs.self_id == m with m
checked against image_id_of(pinned key); every recursing mode asserts the child's self_id or commits it; a fake self_id
gives a journal every reader refuses; zeroconf's check_core_statement requires j.self_id == core_id; hints, utreexo,
zeroconf readers compare core_id with the trusted image); mode confusion (only modes 4/5/8/9/10/16-20, main.rs:2422;
self-test journals cannot start with KIND_CHAIN; trie parts and combines tag-checked); base step, join, anchor
(is_base == 1: anchor_id = link_digest(prev) must equal the pinned genesis anchor; is_base <= 2; a join requires
right.anchor == link_digest(left) and equal lineages; --anchor gives at most DEMO); handover (predecessor id from the
image's LINEAGE; height and lineage asserted; xver_ovm runs verify_full_bytes with the carried key and pin; the reader
accepts zero or lineage_of(stated list)); low work (below min chainwork or the checkpoint: at most DEMO in core,
zeroconf, hints, utreexo, headers); node format (use_format_of per thread, default 2, from the proven profile; profile
== image.profile incl. NODE2); fact checks (zeroconf: all-valid speculative chunk at next height and MTP covering every
input, no same-block coins, every leaf unspent in the tip's coin log; assume: header run end bound to the proven tip,
snapshot by ECMH equality; statement: core_id, height, tip; deposit: core tip bound to its header-MMR leaf, header-only
blocks labelled proof of work only; headers: runs chained from the empty state by input digest); board and coordinator
(verify_any_engine lz_coord.rs:180 uses the board's pinned key, requires self_id, zero lineage, the board's profile, the
start state; server.py passes --check-end in libzync mode, fails closed with an older host; the board is not in a
reader's trust path).

Not covered: OpenVM's verifier and deferral soundness (item 3), the guest's consensus logic (chunk BIP68 / MTP, coin
log internals), the trie-fact verifier beyond its call sites.
