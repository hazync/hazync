# Gate 3, the board half: the real coordinator and worker on the engine (27.11.2026)

engine/STAGE3.md, after 3.2 (the host commands, results/board_engine). The REAL coordinator (`coordinator/server.py`)
and a real worker (`coordinator/hazync`) on hazync-coord's CPU (16 cores, `nice`, RAYON_NUM_THREADS=8), mainnet blocks
1..8, image keys-ch (the engine chain's core image, c7fe0eb85d2d4c10532742191b059a2adcc3e24e22aa81c1e632e2ee718d2fc9,
key pin b5b94f9c...). Everything under /root/gd on hazync-coord, on its own port (127.0.0.1:8911) and its own DB and
directories: the live coordinators on hazync-proof (:8899 / :8900) and their units, DBs and data are not touched.

Scripts (here): `gd_setup.sh <commit>` (build hzengine and the host, prep 1..8 with leaf bundles, the board's key from
`hzengine pack --key`), `gd_run.sh` (coordinator + worker: 8 leaves, folds, spine, check-proofs), `gd_refuse.sh`
(refusals through the real submit path, on a second empty coordinator).

## The coordinator needed no code change

Configured by environment only: the live libzync unit's settings (coordinator/deploy/libzync/hazync-lz-coordinator.service),
its own port and dirs, plus the engine's verify key (3.2): `HAZYNC_ENGINE_VK=<key.bin> HAZYNC_ENGINE_VK_PIN=<pin>`.
With them `host method-id` (so `/api/meta`'s `method_id`) is the engine image, `verify-any` verifies HZE1 files under
the pinned key, and `GENESIS_TIP` is `host libzync-genesis-tip` (852c5c7e..., the RISC Zero board's: the same state
layout). The worker: `HAZYNC_ENGINE=<hzengine> HAZYNC_ENGINE_KEYS=<keys-ch>` (3.2), its own HAZYNC_HOME.

## Measured (commit b1fd6b26 for the run; 99c0572c for the refusals)

| step | result | time |
|---|---|---|
| build (cold): hzengine / the host | | 42 s / 2,513 s |
| `/api/meta` at start | method_id c7fe0eb8..., libzync true, frontier 0 | |
| `hazync run` x 8 (claim the earliest free block, fetch its leaf bundle, prove on the engine, sign, submit; the board re-verifies) | blocks 1..8 each "re-verified your proof and put it on the board"; frontier 8 | 430, 432, 424, 426, 414, 421, 417, 414 s (block 1: prove 252.2 s of it) |
| `hazync fold 8` (pairs the board's /api/foldable offers) | 1+2, 5+6, 3+4, 7+8, 5-6+7-8, 1-2+3-4, 1-4+5-8: 7 folds, each verified on the board; then "nothing to fold" | 1,894 s (271 s per fold) |
| `hazync spine` | seeded with block 1, then absorbed block 2, [3..4], [5..8] (the widest chunk each step): spine [1..8], genesis-anchored, 274,503 bytes, out_tip c22fc56a... | 801 s (4 steps) |
| `check-proofs.py` on the stored proofs | 15 of 15 hash to what was accepted and verify to their records, 0 problems | 0 s (111 per second) |

## Refusals (gd_refuse.sh)

The main board answers "already proven" (409) BEFORE verifying a submission for a block it holds, so refusals go to a
second, empty coordinator (port 8912) with the same configuration, where each submission is verified; the same signing
key, through `hazync submit` (refuse.out, on 99c0572c):

| submission (as range 1) | board's answer | worker exit | time |
|---|---|---|---|
| block 1's proof, a journal byte changed | rejected: not a valid proof (forged/tampered/corrupt): the proof's public values are not SHA-256 of this journal | 1 | 275 ms |
| block 1's proof, a proof byte changed | rejected: not a valid proof: the STARK does not verify against the image's key | 1 | 234 ms |
| another image's proof (the header zAPP's 0..4,031, image 163d229d) | rejected: not a valid proof: the STARK does not verify against the image's key | 1 | 320 ms |
| block 2's proof | rejected: not anchored on the state before (the state before block 1 is genesis, 852c5c7e) | 1 | 279 ms |
| control: block 1's real proof | accepted, frontier 1 | 0 | 260 ms |

The "another build" class (a chain proof whose journal names another image: MISMATCH, worker exit 78) is covered by the
host's engine board test only: no second core engine image's proof exists to submit.

## Found by the dry run, fixed (99c0572c)

The first refusal pass refused every bad submission, but gave a tampered journal byte and a tampered proof byte the
reason "your prover's guest image id (METHOD_ID) does not match this coordinator's", the reason for another build:
verify_any_engine said "METHOD_ID MISMATCH or invalid proof" for every proof that did not verify, and the board reads
"MISMATCH" as another build. So a forgery was reported as a contributor's build mistake, and the worker left with its
guest-mismatch exit (78). Now (prover/host/src/lz_coord.rs `engine_refusal`): MISMATCH only when the proof's journal,
read without trusting it, is a chain state naming another image; a forged, tampered or corrupt proof says so; the
board's own key is a configuration error. The verdict was never affected (refused either way). Tests: the engine board
test now checks each class (tampered journal and proof: "forged, tampered or corrupt", no MISMATCH; a journal naming
another image: MISMATCH; a wrong board pin: configuration error); host suite 108 passed, 1 ignored.

## Left for Gate 3

- The board on GPUs and at the board's scale: the pod image with hzengine and its keys (from B2, pinned by SHA-256),
  workers on rented cards; with 3.3's prep data (the whole chain in node format 2) instead of blocks 1..8.
- An engine-to-engine handover on the board: `verify-any` takes a zero lineage only (one engine image from genesis).
- The tip pipeline (3.1), path services (3.4), the web verifier (3.5): the other halves of the dry run.
- Deploy files: an engine coordinator unit (the libzync unit's settings + HAZYNC_ENGINE_VK / _PIN), beside the old
  one, for when the dry run moves to hazync-proof.
