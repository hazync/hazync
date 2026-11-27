# Security parameters and versions (ROADMAP 4.4; 27.11.2026)

## Versions

| component | version | where pinned |
|---|---|---|
| OpenVM | v2.1.0 (2026-11-04), git tag | engine/host/Cargo.toml, engine/verify-core, engine/platform |
| stark-backend | v2.1.0, git tag (checkout a6b4761 "release: v2.1.0 (#401)") | same |
| proof system | SWIRL (stark-backend v2; WHIR as the PCS), BabyBear base field, BabyBear^4 challenges, Poseidon2 | stark-backend crates/stark-sdk/src/config/mod.rs |
| audit | stark-backend v2.0.0 / OpenVM v2.0.0: zkSecurity report (openvm audits/v2/v2.0.0-zksecurity-report.pdf); v2.1.0's diff since: the open question of ROADMAP 1.7 (the user's, at gate 4) | - |

## The claim, and whether this build is inside it

stark-backend v2.1.0 states its production configurations "target 100-bits of proven round-by-round (RBR) security
with BabyBear as the base field and BabyBear^4 as the extension field" (crates/stark-sdk/src/config/mod.rs), and for
the app VM lists the assumptions under which that holds: log stacked height <= 24, <= 5,000 constraints per AIR,
<= 100 AIRs, <= 1,000 interactions per AIR, <= 30,000 trace columns. hzengine builds with exactly those production
functions (`app_params_with_100_bits_security(h)` with h <= 24 enforced, `hook_params_with_100_bits_security()`,
`AggregationSystemParams::default()`: engine/host/src/engine.rs).

Measured (`engine/host/examples/soundness.rs`: stark-backend's own `SoundnessCalculator::calculate_from_vk` on every
verifying key of a keys dir, which takes the bounds from the KEY, not from the stated assumptions; soundness.out):

| key | AIRs | max constraints / AIR | max interactions / AIR | columns | log stacked height | bits |
|---|---|---|---|---|---|---|
| app VM (core image keys-ch, c7fe0eb8) | 39 | 1,481 | 596 | 6,845 | 24 | 100.1 |
| leaf / internal / internal recursive | 44 | 282 | 85 | 1,783-1,796 | 19-21 | 100.0-100.1 |
| deferral circuit, internal recursive, hook | 42 | 282 | 78-96 | 1,695-1,817 | 19-21 | 100.0-100.1 |
| the reader's key (vk.bin, pinned) | 44 | 282 | 85 | 1,783 | 19 | 100.1 |

The headers image (keys-hdr29, 25b32ac5) gives the same rows. Every component of the estimate (LogUp, GKR, ZeroCheck,
batching, stacked reduction, WHIR) prints 100.0 bits or more; WHIR is the binding one (100.0-100.1). soundness.out
prints one decimal, so a row shown as 100.0 (leaf, deferral leaf) lies in 99.95-100.05: "about 100", not "at least
100" (review R1-6). The app VM is inside every stated assumption (39 of 100 AIRs, 1,481 of 5,000 constraints, 596 of
1,000 interactions, 6,845 of 30,000 columns) and EXACTLY at the stacked-height limit (2^24, the maximum allowed). The parameters are part of the verifying key, and readers pin the key by SHA-256, so a proof made under
weaker parameters does not verify.

What this is not: a review of the calculator or of SWIRL's analysis (that is OpenVM's, and the audit question 1.7).
The number is what the backend's own soundness code computes for these keys. Each figure is per circuit and per proof: the
soundness of the whole recursive composition (a chain proof over every segment, leaf, internal and deferral proof) is a
union over those proofs and so a few bits lower; it is not computed here (review R1-6).
