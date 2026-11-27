# R1-1: compat layer (engine/platform; reviewer report, 27.11.2026, as given)

Verdict: no way found for a malicious prover to make the guest commit a journal a correct run would not produce, or
make env::verify accept a wrong child, provided OpenVM's deferral circuit is sound and the final verifier pins the key
file and the image id. Reading only; nothing edited or run.

1. LOW (defence in depth; verified by reading). `bench-noverify` is kept out of production only by the image id.
   engine/platform/risc0-zkvm/src/lib.rs:158, engine/guest/Cargo.toml.in:46, engine/host/src/engine.rs:445: enabled by
   the env var HZ_GUEST_FEATURES, with no build-time refusal. An image built with it accepts every env::verify
   without a proof, but finalize still reveals the real sha256(journal) (:216). The guest reads self_id from the host
   (main.rs:1612/1723/1769/2133/2500), so a bench-image proof can carry a journal whose self_id equals the production
   METHOD_ID. Safe today: its exe commit differs, so the verifier (sha256(exe||vm) from the pinned key vs the published
   id) and a production parent (id == want at :174) reject it. Breaks if an operator builds the RELEASE key dir with
   HZ_GUEST_FEATURES=bench-noverify and publishes that id and key: every forged chain verifies. Fix: in a bench build
   finalize reveals a domain-separated value (e.g. sha256("bench"||journal)) so it can never match; `hzengine build`
   refuses the feature for non-metering output.
2. INFO (verified). The host chooses the `dry` flag (control chunk bit 1, :75-76; skips every env::verify, :158) in
   every production image. Safe: finalize refuses a dry run (:212); finalize is the only place public values are
   revealed (no other reveal / hint / read_vec in the guest or the edits); it is the last statement of main
   (edits.py:27-28) and main has no early return; so a dry run panics or ends with unset public values; OpenVM's
   deferral verifier requires the child's exit code to be success (verify-stark circuit air.rs:133).
3. INFO (no impact; verified). `32 * n` can overflow at :78 on rv32 with overflow checks off; the commitments come from
   the control chunk's real bytes (:79), and the other use of n (:83) only gets stricter.

Checked and sound: what env::verify binds (:156-178: next host commitment in order; image id recomputed as
sha256(app_exe_commit || app_vm_commit) and compared, little-endian words; child public values collapsed byte-wise with
zero high bytes asserted equal sha256 of the exact journal; DeferredVerifyCircuit against the VM's fixed key; same
app_vm_commit; DEF_IDX 0); order and count of deferrals (too few panics :162, too many panics in finalize :215;
reordering only supplies proofs of the same statement); journal to public values (commit / commit_slice match risc0's
word format; vendored serde byte-identical but WORD_SIZE, all four files diffed; bad bool/char/UTF-8 rejected, u8/u16
range-checked); input handling (take, read_slice, read_slice_ref bounds-checked, buffer never written, misaligned slices
copied); SHA-256 (openvm_sha2; the sha2 shim matches OpenVM's reference zkvm_impl.rs: native LE state words, 4-byte
aligned buffer at offset 32 of a repr(C) struct, aligned blocks passed through, unaligned staged, padding correct for
n >= 56, big-endian output; short_digest_limbs reversed state order correct; swiftsync edit 10 b[8] = ctr, n = 36
matches the old m[32..] = ctr.to_le_bytes(); core-0201 same layout with alignment staging); risc0-crypto shim (LE
transmute behind a compile-time assert; from_bigint keeps the >= p refusal via from_le_bytes's Option).

Not checked (out of scope): that app_vm_commit (vm_or_hook_commit) commits to the deferral configuration and the child
key (claimed by ENGINE.md / VERIFIER.md, not traced through OpenVM's key commitments); the guest's own statement logic
(domain tags, self_id propagation main.rs:1628/2287); the verifier-side key pin (VERIFIER.md: verify_full_bytes code 7).
