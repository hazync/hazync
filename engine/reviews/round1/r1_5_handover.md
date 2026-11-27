# R1-5: handover mode (reviewer report, 27.11.2026, as given)

Verdict: no in-guest path lets a prover get a forged predecessor accepted, GIVEN the embedded key file is the
predecessor's genuine key. The weak point is that provenance is unchecked and rests on the release assembler.

1. MEDIUM (process / trust; high confidence). Key provenance is unchecked, and the claimed build-time check does not
   exist. engine/guest/assemble.sh:45-51 embeds any `*.vk` in HZ_OVM_XVER_KEYS whose file name equals its own SHA-256
   (self-consistency only). The guest's image check (verify-core/src/lib.rs:174) compares image_id_of(key.baseline) with
   p.image; a forged key can carry the genuine baseline with a forged mvk, so it does not authenticate the key.
   main.rs:47's compile-time `carried(v)` only checks the pin is present. HANDOVER.md:62-63 says `image ==
   image_id_of(key)` is "checked at build time and in the guest": only the guest check exists. Attack: a release builder
   given (or mistakenly using) a key with the predecessor's real baseline and a forged mvk names it by its hash and
   writes that hash into the lineage entry's `verifier`; the image id commits to the forged key; that image then accepts
   a "predecessor proof" of any ChainState at first_height-1. Also assemble.sh:91-93 gives ovm-xver builds no pinned
   lock, so third parties cannot reliably rebuild the id to audit which key bytes it embeds. Fix: assemble.sh / build.rs
   require LINEAGE.last()'s `verifier` pin to equal the predecessor RELEASE's published key pin (from its image record /
   reproduce/), assert image_id_of(key) == LINEAGE.last().image and that a carried key exists for exactly that pin at
   build time, and pin a lock for ovm-xver builds.
2. LOW (labelling; high; not exploitable). The lineage `verifier` field means a RISC Zero parameter digest in the
   product and an OpenVM key pin in the engine, with no domain tag; the OpenVM version selecting the verifier
   (xver_ovm.rs:81-86) comes from the key's directory (assemble.sh:49) and is not in the lineage digest. The image id
   commits to both, but a lineage alone does not say which verifier was used. The same key in both dirs: find picks
   v2.1.0 first; a mismatched key fails closed.
3. LOW (test coverage; high). In-guest negatives (handover_step2 hv_neg.out, step3 xv3_neg.out) cover a flipped proof
   byte, a changed journal, another real proof, a wrong-version proof. Not exercised in-guest: a key not matching its
   pin, a genuine pinned key of another image (Refusal::Image), a wrong height, a predecessor lineage mismatch, a
   non-chain journal (the code checks exist).
4. INFO (not reviewed). A same-OpenVM handover (verifier == 0) uses env::verify(p.image, ..) via OpenVM deferral
   (main.rs:2265-2266); its binding of p.image's exe/vm commits is outside this scope.

Checked and sound: pin before use (verify_full_bytes hashes the exact embedded bytes against a pin from source;
include_bytes!, so the image id commits to the key); the version byte fixed in source, the v2.0.2 port changing only
decoding and the baseline field; image_id_of(key) == p.image and prev.self_id == p.image (main.rs:2292); journal binding
(32 public values each <= 255 = SHA-256(journal), lib.rs:180-183; the same bytes decoded as ChainState, kind/version
checked main.rs:2280-2284; profile main.rs:2294); length handling (xver_ovm.rs:63-67); height (main.rs:2295, 2302);
lineage (main.rs:2297, 2383, handover_pin.rs:27-33; is_base==2 fails on an empty lineage, main.rs:2263); zero lineage vs
handover (base step changes anchor_id via link_digest main.rs:644-649; a join requires equal lineages main.rs:2573; the
verifier accepts zero or the full pinned list, swiftsync-verify lib.rs:546); collect mode cannot stand in for a chain
proof (exits non-zero; relies on OpenVM checking the exit code, not re-verified).
