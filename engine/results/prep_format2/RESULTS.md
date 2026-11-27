# Prep in node format 2, from genesis, on hazync-proof (engine/STAGE3.md 3.3; started 27.11.2026)

## What runs

| unit | what | dirs |
|---|---|---|
| hazync-engine-fetch.service | bitcoind blocks 1..977,800 as block_<h>.json, 1,500 ahead of prep, 4 jobs | /srv/bulk/hazync-engine-prep/blocks |
| hazync-engine-prep.service | libzync-prep 1..977,800 from genesis: coin log, counting trie, BIP30 tree, states, history.bin; leaf bundles only at the paper's sample heights | stores on vda: /var/lib/hazync-engine-prep/{cl,trie,out} (symlinked from /srv/bulk/hazync-engine-prep) |

Units and installer: coordinator/deploy/engine/ (f9e310e6). Host: prover/host built on hazync-proof from engine/openvm
263e72d0 (HAZYNC_LIBZYNC=1, RISC0_SKIP_BUILD=1: prep runs no guest; leaf bundles are stamped with the proving image on
READ, prep.rs leaf_read, so this data is not tied to an image id and outlives the Stage 4 freeze). host_lz sha256
b41a5d42...; libzync-prep-run.sh = coordinator/deploy/libzync's (6d084989..., newer than the RISC Zero run's deployed
7c84330a: dry run, refusal to clear stores). Settings: SAVE_EVERY 1000, REPORT_EVERY 1000, MIN_FREE_GB 200,
TRIE_CACHE_MB 12288, HISTORY=1, Nice 19, IO idle. No HAZYNC_PREP_CHECK (its reference states are format 1).

The RISC Zero preps (libzync-prep, libzync-board-prep, their fetchers) stay disabled and their dirs untouched
(/srv/bulk/libzync-prep, /srv/bulk/libzync-board, /var/lib/libzync-fast, /var/lib/libzync-board).

Stop: `systemctl disable --now hazync-engine-prep hazync-engine-fetch` (it resumes from its newest save point on the
next start; `LZ_ROOT=/srv/bulk/hazync-engine-prep LZ_HI=977800 LZ_RUN_DRYRUN=1 .../bin/libzync-prep-run.sh` shows
from where).

## Format 2 checked before the start

A prep of 1..200 with the same binary (scratch dir, removed): state_200.json equals the engine chain's proven state
at 200 (the journal of swiftsync-verify/tests/data/engine200/proof.hze) in every field but the stamped three (self_id,
lineage, profile): kind, version, tip, coin-log roots and leaves, value, work, times, anchor_id, live-coin hash, trie
root, SMT root, block forest. Profile 0xf1 on both (bit 7: node format 2). Its history.bin (23,620 bytes) is
byte-identical to the fixture's history.bin.

## The first hour (first_hour.txt: journal -o short-unix, PREP_PROGRESS every 1,000 blocks)

Started 1795790877.9 (27.11.2026 ~15:48 UTC); block 180,000 at 1795794065.0: **180,000 blocks in 3,187 s**. The
headers follower (RISC Zero headers-prove, nice 19, ~14 of 16 cores) ran alongside for part of it.

| height | blocks/s (window rate) | trie MB | coin log MB | out MB |
|---|---|---|---|---|
| 10,000 | 165.6 | 4 | 3 | 44 |
| 50,000 | 128.9 | 18 | 7 | 222 |
| 100,000 | 148.3 | 149 | 18 | 463 |
| 130,000 | 25.1 | 619 | 80 | 620 |
| 150,000 | 32.9 | 1,180 | 296 | 824 |
| 170,000 | 24.7 | 1,592 | 361 | 1,005 |
| 180,000 | 16.7 | 2,315 | 405 | 1,106 |

Disk at 180,000: cl 456 MB, trie 2.3 GB, out 1.2 GB (states + history), blocks window 366 MB. Growth per 1,000 blocks
between 170,000 and 180,000: trie +72 MB, coin log +4.4 MB, out +10 MB, and it grows with height.

## For comparison: the RISC Zero run's measured rates (format 1, the same box; journal of libzync-prep)

The rate falls steeply with height: the stores outgrow RAM and the work becomes random 4 KiB I/O
(project_libzync_prep_throughput). Measured window rates of the run from genesis that began 16.11.2026: 100,000:
371 blocks/s; 150,000: 19.4; 200,000: 12.9; 300,000: 3.8; 400,000: 0.45; 500,000: 0.16; 548,000: 0.14 (it was
stopped at 548,250 on 26.11, about 10 days after it began; trie 184 GB, coin log 36 GB at 548,000). No flat ETA is
given for this run: its rate at each height is measured as it gets there.
