# Round 1: assessment of every finding (27.11.2026)

Each finding: checked against the code by me (not taken on the reviewer's word) where it claims an exploit; the cause;
every path it affects; the decision. Severity as I assess it, which may differ from the reviewer's.

## Confirmed exploitable (fixed in 0382bb3f; verification running: r1_3_verification.md)

| # | finding | checked | cause / paths | decision |
|---|---|---|---|---|
| R1-3 #1 | hinted R = (0,0) -> identity -> forged BIP340 (r = 0) passes the batch | YES: OpenVM v2.1.0 weierstrass.rs:108-117 `from_xy(0,0) = Some(IDENTITY)`; batch_ecdsa_ovm.rs used from_xy; accept_r kind 1 checks only x == rx and y even | the port dropped the product's y^2 == x^3 + 7 check; every deferred kind (1 exploitable; 3/4 need P + t.G = O) | FIXED: from_xy_nonidentity (checks the curve, refuses the identity) |
| R1-3 #2 | R.y not in the batch transcript: for ECDSA the prover picks +/-R after the weights (k-tree ~2^40) | YES: engine hashed 160 bytes per item (no R.y); the product hashes its whole repr(C) Deferred incl. r: Pt (192 bytes) | the port's raw layout | FIXED: raw = u1, u2, Q.x, Q.y, R.x, R.y (192 bytes); header corrected (it claimed "SAME transcript") |
| R1-3 #3 | MSM window c = 16: i16 digit wraps | YES (code: 1..=16 with i16 digits) | liveness only (valid batch rejected) | FIXED: c <= 15 |

## To fix (not exploitable today)

| # | finding | assessment | decision |
|---|---|---|---|
| R1-6 #1 (DONE, fixes_verifier.md) | CLIs pin a key to its own hash ("taken as your own build's"): a forged key next to a proof passes | correct; harmless only while no image is published and readers are told; a release must not ship it | require a separately given pin (`--vk-sha256`) when the record has no key; docs: the image id alone adds nothing |
| R1-6 #2 (DONE) | engine/wasm-verify exports the unpinned STARK-only check as valid=1 and an image id from the (forgeable) key | correct (a probe module, not the published one, but exported) | rename / remove the unpinned exports; fix the "compressed" doc line |
| R1-6 #3, #4 (DONE) | zeroconf decodes the journal without decode_journal's %4 guard | unreachable after verify_full_bytes; cheap | use decode_journal; also check core.has_image() (#4) |
| R1-5 #1 | handover: nothing checks the embedded predecessor key is the predecessor RELEASE's published key; HANDOVER.md claims a build-time check that does not exist | correct; trust rests on the assembler | build-time check: LINEAGE.last().verifier == the published predecessor pin and image_id_of(key) == LINEAGE.last().image; correct HANDOVER.md; pinned lock for ovm-xver builds |
| R1-5 #3 | in-guest handover negatives missing (key != pin, another image's key, wrong height, lineage mismatch, non-chain journal) | correct | add them to the handover negatives |
| R1-1 #1 | bench-noverify kept out of production only by the image id | correct; an operator error would publish a forgeable image | finalize reveals a domain-separated value in bench builds; hzengine build refuses the feature for a release keys dir |
| R1-2 #1 | edits.py edit 4's anchor not asserted unique | correct (lands right today) | assert it like the other edits |
| R1-4 A1 | the C heap turns a latent use-after-free / double free in our C++ into live aliasing | a risk, no instance found; the reviewer names cached_tx / hz_cache_reset to look at | run the heap-poison runs over the vectors and edge blocks; inspect cached_tx's lifetime; a stronger double-free check (generation tag) to consider |
| R1-4 B3 | producers never set the node format (inherit the thread's last) | liveness only | producers set format 2 explicitly |
| R1-3 #4 | a failed batch sets all_valid = false, which a prover can force | correct; "not proven", never "invalid" | document; check nothing treats it as proof of invalidity |

## Documentation only

R1-2 #3 (coinlog_ovm.py:47 comment), R1-4 A4 (cshims.c:80-81 bad_alloc comment), R1-6 the security_params caveats
(one decimal; per circuit, not the recursive composition; the app VM at the height limit): reword RESULTS.md.

## Noted, no change

R1-1 #2, #3; R1-2 #2 (exit panics: fails safe); R1-4 A2, A3 (re-check A3 on the engine ELF), A5, B1, B2; R1-5 #2 (a
domain tag for the lineage `verifier` field: consider at the next lineage format change); R1-6 #4.

## Not covered by round 1 (for round 2)

OpenVM's own claims every report relied on: the verifier accepts only exit code 0; app_vm_commit binds the deferral
configuration and child key; the same-OpenVM handover's env::verify binding (R1-5 #4). And the guest's statement logic
(domain tags, self_id propagation) outside the engine-specific code.
