# Hazync documentation

Bitcoin Core's own consensus code, executed inside a RISC Zero zkVM, so that a block's validity can be **proven once
and verified by anyone** without re-executing it. **Hazync v2** is the libzync engine (`libzync/`, the guest in
`prover/methods/guest`), on RISC Zero 3.1.0 since release v0.24.0: published images (v0.26.0) mainnet `1ff5a129`, which continues v0.24.0's
`8888d59c` at block 2000 by a handover ([`HANDOVER.md`](HANDOVER.md)), signet `28e39600`, and the header-chain zyncAPP `2db34e12` ([`../reproduce/LIBZYNC_IDS`](../reproduce/LIBZYNC_IDS)). The
public board has run it since 2026-11-12, from genesis.

**Hazync v1**, the original per-signature CORE guest (`37987b85`, last release v0.22.1), is documented in
[`history/`](history/README.md) and its code is in [`../history/`](../history/README.md). Those pages are the record
of the past and are not kept current.

## Start here

| | |
|---|---|
| [`../README.md`](../README.md) | what Hazync is and where it stands |
| [`EXPLAINER.md`](EXPLAINER.md) | Hazync v2 in plain words |
| [`../CONTRIBUTING.md`](../CONTRIBUTING.md) | prove blocks on your GPU (Linux, Windows natively or via WSL2), verify a proof |
| [`RELEASE_NOTES_v0.25.0.md`](RELEASE_NOTES_v0.25.0.md) | the current release: the Windows installer; images unchanged |
| [`RELEASE_NOTES_v0.24.0.md`](RELEASE_NOTES_v0.24.0.md) | RISC Zero 3.1.0, the new image ids, the header chain in one proof |
| [`RELEASE_NOTES_v0.23.0.md`](RELEASE_NOTES_v0.23.0.md) | the first Hazync v2 release, **withdrawn** (RISC Zero 3.0.5); kept for what it says about the switch |
| [`../CHANGELOG.md`](../CHANGELOG.md) | every release, newest first |

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

## Soundness and review

| | |
|---|---|
| [`SOUNDNESS.md`](SOUNDNESS.md) | what a Hazync v2 proof rests on, and what would break it |
| [`EXTERNAL_REVIEW.md`](EXTERNAL_REVIEW.md) | a guide for an outside reviewer: where to start, what to try |
| [`../SECURITY.md`](../SECURITY.md) | security policy, reporting, supported versions, review status, where review is worth most |

## Running it

| | |
|---|---|
| [`RUN_YOUR_OWN_COORDINATOR.md`](RUN_YOUR_OWN_COORDINATOR.md) | standing up your own Hazync v2 proof party |
| [`COORDINATOR.md`](COORDINATOR.md) | the coordinator: running your own, sponsorship, the sponsor bot |
| [`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md) | every coordinator route and setting, and the worker CLI, **generated from the code** (`scripts/gen-coordinator-reference.py`) |
| [`OPERATIONS.md`](OPERATIONS.md) | the operator's manual: §0 the live Hazync v2 board, §3 board workers (both boards); the rest is the Hazync v1 fleet's reference (topology, multi-card fleets, tip runs) |
| [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md) | builds, image ids and reproducibility, cutting a release |
| [`../coordinator/deploy/RUNBOOK.md`](../coordinator/deploy/RUNBOOK.md) | the public board's own box, day to day |
| [`../coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md) | how the board moved from Hazync v1 to v2 on 2026-11-12 |
| [`../coordinator/deploy/headers/README.md`](../coordinator/deploy/headers/README.md) | the header-chain follower that keeps the header proof at the tip |
| [`../prover/README.md`](../prover/README.md) | the prover: layout, build, host commands |
| [`../swiftsync-verify/README.md`](../swiftsync-verify/README.md) | the standalone verifier: verdicts, options, test vectors |
| [`../tools/win-gui/README.md`](../tools/win-gui/README.md) | the Windows GUI for the prover |

## History: Hazync v1

| | |
|---|---|
| [`history/README.md`](history/README.md) | the development record, with its known-stale figures |
| [`history/OVERVIEW_CORE.md`](history/OVERVIEW_CORE.md), [`history/SPEC_CORE.md`](history/SPEC_CORE.md), [`history/SECURITY_MODEL_CORE.md`](history/SECURITY_MODEL_CORE.md), [`history/SECURITY_CORE.md`](history/SECURITY_CORE.md) | the original CORE system: overview and glossary, its normative spec (checked by `history/scripts/check-spec.sh` while CORE was current), threat model, and review status (moved 2026-11-08) |
| `history/*.md` | the 23 September-era documents merged in 2026-11-06, unchanged (e.g. `history/SOUNDNESS.md`, `history/PROVING.md`), plus the dated `STATUS`, the v0.21.7 pre-release trial, the field-backend and lift-x design notes. `history/EXPLAINER.md`, `history/SOUNDNESS.md`, `history/EXTERNAL_REVIEW.md` and `history/RUN_YOUR_OWN_COORDINATOR.md` are the Hazync v1 versions of the pages of the same name above |
| [`history/releases/`](history/releases/) | copies of Hazync v1's release bodies (to v0.22.1) |
| [`history/SECURITY_AUDIT_LOG.md`](history/SECURITY_AUDIT_LOG.md) | the round-by-round security review record, indexed from `../SECURITY.md` |
| `../experimental/` | experiments and their results; `../history/experimental/libzync-bench/retired-docs/` holds libzync's retired design-B documents |

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
