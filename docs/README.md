# Hazync documentation

Bitcoin Core's own consensus code, executed inside a RISC Zero zkVM, so that a block's validity can be **proven once
and verified by anyone** without re-executing it. **libzync** is the new core that proves every block from genesis
(`libzync/`); the original Hazync system (the canonical CORE guest, the coordinator, the fleet) is documented here
and still runs.

The docs were condensed on 2026-11-06: the 23 September-era documents were merged into the six below, and every
original is kept unchanged in [`history/`](history/README.md), the development record, which **must not be quoted
for numbers** without its corrections. Every release is in [`../CHANGELOG.md`](../CHANGELOG.md); the GitHub
releases are canonical.

## libzync, the new core

| | |
|---|---|
| [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) | what it proves, how, what it measured, its limits: start here |
| [`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) | the proof statement (`ChainState`) and every check a verifier makes (the name is historical) |
| [`libzync/CONSENSUS_RULES.md`](libzync/CONSENSUS_RULES.md) | Core's consensus rules, one by one: enforced, how, and by which test |
| [`libzync/COINLOG.md`](libzync/COINLOG.md) | the coin log: coin accounting sound at every block |
| [`libzync/UTREEXO_FORMAT.md`](libzync/UTREEXO_FORMAT.md) | the byte-exact coin leaf and the forest hashing (the name is historical) |
| [`libzync/HANDOVER.md`](libzync/HANDOVER.md) | the block forest, and how a new image continues a chain |
| [`libzync/ZYNCAPPS.md`](libzync/ZYNCAPPS.md) | zyncAPPs: separate guests that answer one question from a core proof |
| [`libzync/PROOF_BUNDLE.md`](libzync/PROOF_BUNDLE.md) | the proof bundle format |
| [`libzync/ROADMAP.md`](libzync/ROADMAP.md) | everything built, measured and still open, in order |

## The Hazync system

| | |
|---|---|
| [`OVERVIEW.md`](OVERVIEW.md) | what Hazync is, its goals, how a block becomes a proof, and the glossary |
| [`OPERATIONS.md`](OPERATIONS.md) | topology and settings, proving a block, running provers, the GPU fleet, tip runs; states the current release |
| [`COORDINATOR.md`](COORDINATOR.md) | the coordinator: running your own, sponsorship, the sponsor bot |
| [`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md) | every coordinator route and setting, and the worker CLI, **generated from the code** (`scripts/gen-coordinator-reference.py`) |
| [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md) | builds, image ids and reproducibility, cutting a release |
| [`SECURITY.md`](SECURITY.md) | what a proof guarantees and assumes, the threat model, soundness gaps, durability, review status, fuzzing |
| [`SPEC.md`](SPEC.md) | the CORE guest's specification (normative; `scripts/check-spec.sh` checks it against the code) |
| [`../SECURITY.md`](../SECURITY.md) | security policy, reporting, and the audit rounds |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | contributing GPU time to the board |
| [`../prover/evidence/README.md`](../prover/evidence/README.md) | index of the committed evidence files, with provenance and known issues |

## History

| | |
|---|---|
| [`history/README.md`](history/README.md) | the development record, with its known-stale figures |
| `history/*.md` | the 23 documents merged above, unchanged (e.g. `history/SOUNDNESS.md`, `history/PROVING.md`), plus the dated `STATUS`, the v0.21.7 pre-release trial, the field-backend and lift-x design notes |
| [`history/releases/`](history/releases/) | copies of release bodies |
| [`history/SECURITY_AUDIT_LOG.md`](history/SECURITY_AUDIT_LOG.md) | the round-by-round security review record, indexed from `../SECURITY.md` |
| `../experimental/` | experiments and their results; `../experimental/libzync-bench/retired-docs/` holds libzync's retired design-B documents |

## ⛔ How to read a number in this repository

Every performance figure should say **what was measured, on what, and with what still unmeasured.** This project
has repeatedly been wrong by believing a projection, and the corrections are recorded rather than quietly edited out:

- a bigint2 projection of **7.53x** on the tip block measured **4.48x** (`history/TIP_BLOCK_BIGINT2_2026-08-28.md`)
- a field-backend projection of **3.67x** first measured **2.381x**: limb copies at the Rust/C boundary cost
  2,191 M cycles, 38% of the block, and removing them gave **3.836x** (`history/FIELD_BIGINT2_BACKEND.md`)
- `fe_sqrt` sized at **9.83%** from a flat profile was **6.17%** cumulative once the field ops beneath it were
  accelerated (`patches/0013` header)
- a packer whose model priced every chunk identically had an actual straggler of **1.563x** on real cycles, worse
  than not packing (`prover/host/src/main.rs`, packer notes)
- a CORE card count of **10** L40S, computed from measurements on two cards, became **~13** once an eight-card fleet
  was run (`history/BUILDS.md` §1, `history/BENCH_8xL40S_2026-09-08.md`)
- libzync's c14 against CORE is **~8.2x ESTIMATED** in card time, by chaining two measurements on different cards; the
  8.6x is design B's, measured on one A40 (`libzync/LIBZYNC.md` §4)

**If a figure does not say how it was obtained, treat it as a projection.**
