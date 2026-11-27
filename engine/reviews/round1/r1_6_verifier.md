# R1-6: verifier, parameters, segpar (reviewer report, 27.11.2026, as given)

No way found to forge a proof or get a verdict a correct proof should not have, PROVIDED the verifying key the reader
uses is trustworthy; nearly everything is about where that key comes from. Read-only; also read OpenVM v2.1.0's
verifier source (05f3b73, crates/verify/src/{lib,codec}.rs).

1. MEDIUM (high confidence). The CLIs compute the "pin" from the key file they were just given, so it binds nothing.
   swiftsync-verify/src/main.rs:134-141 (with_engine_key hashes the bytes given, lib.rs:147-151),
   zapps/headers/cli/src/main.rs:72, zapps/hints/cli/src/main.rs:72, zapps/utreexo/cli/src/main.rs:122,
   zapps/zeroconf/cli/src/main.rs:70, corecheck::with_key with pin = None (zapps/corecheck/src/lib.rs:100). Scenario:
   the prover ships proof.hze plus a vk.bin with a forged mvk and the genuine baseline copied in; the user types the
   published image id; image_id_of matches (from the copied baseline); the STARK verifies under the forged mvk; a
   made-up journal passes; verdict OK. The code prints "taken as your own build's" and no image is published yet, so a
   UX / protocol risk, not a code bug. Until IMAGES embeds keys: require a separately supplied --vk-sha256 (or refuse a
   key next to the proof); docs to say the image id adds no security without the key.
2. LOW-MEDIUM (high). engine/wasm-verify/src/lib.rs:17-42 still exports the unpinned STARK-only check as "valid = 1"
   (verify / verify_proof -> verify_stark_bytes, verify-core lib.rs:116, no pin), and image_id_hex (:52) shows "the
   image id the vk commits to", which a forged vk sets at will. A page using them gets no assurance. The module doc says
   "compressed encoding"; the code decodes the plain one. Fix: remove the exports or rename them *_unsafe.
3. LOW (medium). zapps/zeroconf/src/lib.rs:102 calls journal.decode() without decode_journal's %4 guard; the decoder
   (engine/journal/src/serde/deserializer.rs:87) panics on a word-aligned buffer whose length is not a multiple of 4
   (abort in wasm/C). Not reachable (after verify_full_bytes the journal is the guest's word output); use decode_journal.
4. INFO. zeroconf does not check core.has_image() (a no-image record compares core_id with zeros: harmless); zeroconf
   takes the profile of the caller's core image with no coin-log-bit check (built-in records set it).

Checked and sound: verify-core (pin before any decoding, lib.rs:189; vk and image id before the proof; unpack refuses a
0 or overlong proof length; OpenVM's decode refuses trailing bytes; public values count vs num_user_pvs and Merkle proof
vs final_root, each canonical and <= 255, exactly 32 via the Option<Vec> compare; program commitment, initial memory /
pc, exit code, every vk commitment and the deferral fields checked against the pinned baseline; the vm hash omits
internal_recursive_vk_commit, irrelevant with the key pinned); statement_checks (kind, version >= 2, self_id == image;
exact profile; time-too-new saturating; forest shape; lineage zero or the image's; anchor genesis or given -> DEMO;
coin-log bit required; live-hash / trie-root zero when off; OK needs no anchor, min work AND height >= checkpoint; a
no-image / no-key record FAILs before decoding; use_format_of per thread, reset by every fact check); malformed input
(hex digits checked first; get() throughout; tx counts capped 2^20, compact sizes at Core's limits; Merkle depth <= 31
with index vs branch length; forest / coin-log roots <= 64, set bits, path length; trie decode depth <= 256 per layer;
serde Vec cautious size hints; ChainState has no strings or buffers; proof_journal framing right; no panic path found);
segpar (only feeds OpenVM's aggregation; worker segment proofs checked in the leaf circuit's continuity checks; its own
baseline used only for the host's own check, engine.rs:386,412).

The 100-bit claim is supported as written, with caveats: soundness.out prints one decimal, so "100.0" (leaf and
deferral leaf, WHIR binding) could be 99.95-100.05 ("at or above 100" not quite established); the figure is per circuit
and per proof, not the whole recursive composition (a union over layers and segment proofs: a few bits fewer); the app
VM sits exactly at the log stacked height limit of 24. "Pinned key => weaker parameters do not verify" is correct (the
parameters live in mvk).
