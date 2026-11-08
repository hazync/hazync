# Hazync documentation

Bitcoin Core's own consensus code, executed inside a RISC Zero zkVM, so that a block's validity can be **proven once
and verified by anyone** without re-executing it. **libzync** (`libzync/`, the guest in `prover/methods/guest`) is
Hazync's core: published images mainnet `863c9ce3`, signet `73b62530` ([`../reproduce/LIBZYNC_IDS`](../reproduce/LIBZYNC_IDS)).
It replaced the original per-signature CORE guest (`37987b85`, documented in [`history/`](history/README.md)).

## libzync: what it proves and how

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

## Running it

| | |
|---|---|
| [`COORDINATOR.md`](COORDINATOR.md) | the coordinator: running your own, sponsorship, the sponsor bot |
| [`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md) | every coordinator route and setting, and the worker CLI, **generated from the code** (`scripts/gen-coordinator-reference.py`) |
| [`OPERATIONS.md`](OPERATIONS.md) | topology and settings, proving, running provers, the GPU fleet; states the current release (still written for the CORE fleet: being revised) |
| [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md) | builds, image ids and reproducibility, cutting a release |
| [`../SECURITY.md`](../SECURITY.md) | security policy, reporting, review status, where review is worth most |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | contributing GPU time |

## History

| | |
|---|---|
| [`history/README.md`](history/README.md) | the development record, with its known-stale figures |
| [`history/OVERVIEW_CORE.md`](history/OVERVIEW_CORE.md), [`history/SPEC_CORE.md`](history/SPEC_CORE.md), [`history/SECURITY_MODEL_CORE.md`](history/SECURITY_MODEL_CORE.md), [`history/SECURITY_CORE.md`](history/SECURITY_CORE.md) | the original CORE system: overview and glossary, its normative spec (`scripts/check-spec.sh` still checks it), threat model, and review status (moved 2026-11-08) |
| `history/*.md` | the 23 September-era documents merged in 2026-11-06, unchanged (e.g. `history/SOUNDNESS.md`, `history/PROVING.md`), plus the dated `STATUS`, the v0.21.7 pre-release trial, the field-backend and lift-x design notes |
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
- libzync against CORE was **~8.2x ESTIMATED** in card time by chaining two measurements on different cards; measured
  on one card and real mainnet state it is **7.0x** over blocks 200,000-300,000, from 2.7x on a small block to 15x
  (`../experimental/libzync-bench/results/CYCLES_REAL_STATE.md`)

**If a figure does not say how it was obtained, treat it as a projection.**
