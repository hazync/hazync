# Reorg handling for the libzync board (design)

> **Design, 16.11.2026; P1 and P2 built the same day** (branch `feat/reorg-p1-p2`, §15). The rest (P3 to P6) is not
> built. I wrote it from the repo at `797b777e` (main) and a read-only survey of `hazync-proof` the same day.
> **MEASURED** numbers come from that survey or the files named next to them. **ESTIMATE** numbers are labelled. The
> user's decisions on the open questions are in §14; what building P1 and P2 corrected in this page is in §15.

The board is about to follow the chain tip. When it does, some blocks it has fetched, prepped and proved will be
orphaned by a reorg. This page covers how each stage notices that, how the board undoes the orphaned blocks on its
own within a window, and what happens past that window.

## Decisions at a glance

| | proposal |
|---|---|
| automatic window **W** | **288 blocks** (Core's `MIN_BLOCKS_TO_KEEP`; the v1 bridge used the same, `bridge.rs` `HAZYNC_BRIDGE_UNDO_KEEP`) |
| undo journal kept **J** | **2,016 blocks** (2 weeks). It costs about 1 MB per block (ESTIMATE, §6), so keeping more than W is cheap. A fork deeper than W but within J **stops and alerts**, and a person then recovers with **one command**, without restarting prep from genesis |
| beyond J | stop and alert; the only sound recovery is prep from genesis (§11). **Decided:** no cold copies of the stores |
| the tip run | **decided:** the paper prep (`libzync-prep`, `LZ_ROOT=/srv/bulk/libzync-prep`) follows the tip (§3) |
| source of truth | the **board's prep run**: its `state_<h>.json` files say which chain the board proves. The **block fetcher** is the only stage with RPC, so it **detects** the reorg; **prep** carries it out; the **coordinator** follows prep's reorg records |
| prep rewind | **per-block undo records** for the coin log, the trie and the BIP30 tree (§6, option B). Every piece already exists in the code, except the trie's undo across sessions |
| coordinator | orphaned rows and files are **moved aside as records, not deleted**; the spine falls back to a kept head; a hash check when a submission commits closes the in-flight race |
| confirmation margin | **none (decided)**. A depth knob for claims exists if wanted (`TIP_CLAIM_DEPTH`, default 0). Offsite copies wait **6 blocks** (§9, decided) |
| image id | **unchanged.** No guest change. No change to any on-disk format a v0.26.0 prep run writes (§1) |

## 1. Does this change the image id?

**No.** Nothing in this design touches guest code, and no part of it changes an on-disk format that a prep run
started now under the v0.26.0 host (`46666b6f`) writes. **v0.26.0 does not need to be held for this.**

The guest is `prover/methods/guest/` plus the files it includes by `#[path]`. From libzync those are `wire.rs`,
`net.rs`, `coinlog.rs`, `coinlog_forest.rs` and `ttree.rs`; from coinbase-smt, `roots.rs` and `bip30.rs`. I checked
with `grep '#\[path' prover/methods/guest/src/`. `libzync/src/trie_store.rs` is **not** in the guest (its header
says it is host only, and no `#[path]` names it), and neither is `libzync/src/host/` (`std` only).

| component | change | guest code? | moves the method id? |
|---|---|---|---|
| guest | none. It already binds every block to the previous tip (`prevhash_ok`, `main.rs:1650/1728/2302`, asserted at `:1670/:1736/:2325`) and every join to the left side's end state (`main.rs:2501`) | — | no |
| host: prep (`prep.rs`, `lz.rs`, `trie.rs`, new `rewind.rs`) | write undo records; a `libzync-prep-rewind` command; a follow mode | no | no |
| `libzync/src/trie_store.rs` | `undo` for a block applied in an earlier session, using a journaled `replaced` flag (today it refuses those) | no (host only) | no |
| host: `lz_coord.rs` (`verify-any`) | none needed: it already reads the board's state files, which the rewind moves | no | no |
| coordinator `server.py` | reorg records, ledger rewind, spine heads, a hash check at commit, API fields | no | no |
| scripts: `fetch_block_rpc.py`, `libzync-prep-run.sh`, `hazync-lz-zapp-follow.sh`, `hazync-offsite-proofs.py` | follow mode and detection; rewind on restart; rewind instead of `die`; depth delay and orphan markers | no | no |
| units | `follow` instead of the static `LZ_HI`; `RestartPreventExitStatus=3 4`; an `alert.conf` (`OnFailure=hazync-alert@%n`) on the prep units | no | no |

⚠ **One build caveat.** A host carries the guest ELF it embeds. A host built through the release pipeline from a commit
whose guest sources are unchanged embeds `46666b6f`. A host built on the laptop with another rustc can embed a
different guest (memory: host rustc 1.100.0 moved the libzync ids). Build these hosts the release way, then check
`host method-id` before installing one.

**On-disk state.** Every existing format stays as it is: coin log `L*.bin` / `index.bin` / `pending.bin` (LZPEND03),
the trie's `nodes.bin` / `meta`, and `state_<h>.{json,smt,coinlog,trie,forest,flags}`. The design only **adds**
files: a `reorg/` directory of undo records, a `REWIND` intent marker, `orphaned/` directories, and
`spine/heads/`. A v0.26.0 run continues under the new host from its newest save point (`HAZYNC_PREP_CONTINUE=1`, as
for any host change that keeps the image). The undo window then fills from that moment: W = 288 blocks takes about
20 min at the measured 4 s per block. The only requirement is that the new host is running before prep comes within
W of the tip. The paper prep is about 344,000 blocks below the tip today (§2), so there are weeks to spare.

## 2. Facts this rests on (box survey, 16.11.2026)

| | MEASURED |
|---|---|
| node tip | 976,104 |
| paper prep (`libzync-prep`, `/srv/bulk/libzync-prep`, out `/var/lib/libzync-fast/out`) | save point 632,000; 0.21–0.26 blocks/s around 630,000–632,000 (`PREP_PROGRESS`), so about 4–5 s per block; a save takes 142–298 s of each 1,000-block interval |
| store sizes at 632,000 | coin log 55 GB, trie 290 GB (4.0 billion nodes), `.smt` snapshot 22.8 MB, `state_<h>.json` 3.7 KB, **one state file per height kept** (632,587 files) |
| coin-log journal | `pending.bin` = 215,116,504 bytes for the 587 blocks since the 632,000 save, so **366 KB per block** |
| block files | `block_633520.json` 3.2 MB; 1,508 files waiting = 4.4 GB |
| board pass (`libzync-board-prep`) | at 5,998, `LZ_HI=974936` (static), 5,000 leaf bundles in its window |
| coordinator | state dir (`HAZYNC_LZ_STATE_DIR`) `/var/lib/libzync-board/out`, bridge `/srv/bulk/libzync-board/leaves`; `spine.bin` 229 KB; 3,206 proofs, median 228 KB |
| disks | `/` 1.4 TB free, `/srv/bulk` 3.4 TB free |

From the code: prep asserts that each block extends the state's tip (`prep.rs:197`), so today a reorg crash-loops it
under systemd. The disk trie is copy-on-write: slots freed since the last flush are held back, so a crash leaves the
last flushed trie intact (`trie_store.rs` header, `DiskArena::durable`). The coin log is changed in place, and
`pending.bin` records undo it back to the last save, but `cl_journal_commit` deletes them at each save
(`lz.rs:1449`). `smt_advance_ex` already returns each touched key's prior value (`prover/host/src/main.rs:1159`, #484), and prep
throws them away (`prep.rs:305`, `.0`). `TrieStore::undo` exists, but refuses any block not applied in the current
session (`trie_store.rs:866`).

## 3. Which run follows the tip (decision D1, the user's call)

The box runs two prep pipelines. A reorg only matters to a run that comes within J of the tip.

- **Board pass** (`libzync-board-*`, `/srv/bulk/libzync-board`): the coordinator's bridge, paced by proving through
  `HAZYNC_PREP_LEAF_WINDOW=5000`. It reaches the tip only once the board has proven nearly the whole chain.
- **Paper prep** (`libzync-prep`, `/srv/bulk/libzync-prep`): not paced. It is the only run that will reach the tip
  soon (ESTIMATE about 16 days from 632,000 at the measured rate, if it is not restarted).

The mechanism is identical for both: it lives in the host and the scripts, and any run started with `follow` gets it.
A run with a static `LZ_HI` at least J below the tip needs no rewind. It still gets the detection check (§4), which
turns a reorg deeper than its distance from the tip into a stop, never a silent wrong chain. **What I need decided:**
does tip proving read bundles and states from the paper run (the coordinator then needs a second bridge and state
directory above some height), or does the board pass switch to `follow` once it catches up? This page assumes "the
tip run" is whichever one it is. **Decided (16.11.2026): the paper prep is the tip run** (§14).

## 4. Detection

**One comparison catches a reorg of any depth.** If the node's active chain has prep's tip hash at prep's tip height,
then every block below it is on the active chain too, because headers chain by hash. If it does not, there was a
reorg at or below prep's tip, however deep.

| stage | how it learns | what it does |
|---|---|---|
| **fetcher** (`fetch_block_rpc.py follow`, the only stage with RPC) | each poll (every 5 s): `getbestblockhash`. When that changes, it walks back by `previousblockhash` from the node's tip until it meets a height ≤ prep's tip whose hash equals prep's `state_<h>.json` `tip_hash` (the **fork F**). It also checks each waiting `block_<h>.json` against `getblockhash(h)` | fork above prep's tip: it deletes and refetches the waiting files above F (nothing else changes). Fork below prep's tip: it writes `blocks/REORG` (F, old and new hashes F+1..) and stops fetching. No agreement within J: it writes `blocks/REORG_TOO_DEEP` (§11) |
| **prep** (follow mode) | between blocks, it checks for `blocks/REORG` (it already polls that directory every 200 ms). It also replaces the `prep.rs:197` assert with a check: a block whose `prev` is not its tip means "reorg suspected", so it waits for the fetcher's verdict instead of panicking | runs the protocol in §5. Exits with code **4** when the fork is deeper than W |
| **coordinator** | a thread polls prep's `reorg/records/` every 2 s, and checks at start-up before binding its port. Records carry an increasing **epoch** | rewinds its ledger (§7), writes `<epoch>.ack` |
| **in-flight submissions** | at **commit**, under `_lock`, it checks that `in_bhash` = `tip_hash` of `state_<lo-1>.json` and `out_bhash` = `tip_hash` of `state_<hi>.json` (the files as they are now) | refuses with **409 `reorged`**. This closes the race where the STARK check (up to 120 s, outside the lock) read the old files |
| **spine submissions** | the same check at commit in `submit_spine` | 409 |
| zAPP follower, offsite copies, website | through the coordinator (spine, `/api/meta.reorg`) | §9, §10 |

Today the comparison against prep's states is the trust boundary: the board proves what its own node says. A node
that is itself on a minority chain is out of scope here. The proofs stay sound either way (§8); the node only decides
which valid chain the board proves.

## 5. The protocol (fork F, epoch E)

Order matters. Nothing above F is served again until the ledger has dropped the orphaned rows, and nothing new above
F is published until prep's stores are back at F.

1. **The fetcher** writes `blocks/REORG` and stops.
2. **Prep** stops applying blocks and writes the intent marker `out/REWIND` (F, E, the orphaned hashes) atomically.
3. **Prep moves its served files above F** into `out/orphaned/E/`: the `leaf_<h>.bin` bundles, then the
   `state_<h>.json` files (with their markers), and the block files. From this instant the coordinator **fails
   closed** above F: `verify-any --check-end` finds no state there (`lz_coord.rs` `state_before`), and claims stop at
   F once `_servable_high(force=True)` runs.
4. **Prep writes `reorg/records/E.json`**: F, depth, the old and new hashes, and the time.
5. **The coordinator rewinds** (§7) in one transaction, then writes `reorg/records/E.ack`.
6. **Prep waits for the ack.** It does not move on without it: new chain files above F next to orphaned ledger rows
   would give 409 "already proven" collisions, and the frontier would follow the orphaned branch. After
   `REORG_ACK_TIMEOUT` (10 min) it alerts and keeps waiting.
7. **Prep undoes its stores** to F (§6), writes a complete save point at F, deletes save points above F, cuts the undo
   records above F, and deletes `out/REWIND`.
8. **The fetcher resumes** at F+1 on the new chain, and prep carries on forward.

**Crash safety.** A start that finds `out/REWIND` finishes the rewind before anything else. Every step is idempotent:
moves skip files already moved, the coin log undo is idempotent already (`cl_undo_to`), and the trie undo starts from
the last **flushed** trie, which is unchanged until step 7's flush. The coordinator step is keyed by E.

## 6. Prep rewind

Prep's state has six parts: `ChainState` (forest, times, ECMH), the coin log, the trie, the BIP30 tree, the leaf
bundles, and `history.bin` when it is written. `ChainState` costs nothing to restore: `state_F.json` is on disk for
every height, so the ECMH, block forest and recent times all come from it. Bundles and history are files to move or
truncate. **The coin log, the trie and the BIP30 tree are what make this hard.**

| option | how | disk | time for a depth-d rewind | code | verdict |
|---|---|---|---|---|---|
| A. rolling full save points in the last W | copy the stores at each save | 345 GB per copy (55 + 290, MEASURED) | copying alone takes hours (ESTIMATE: about 2.4 h at the ~40 MB/s `/srv/bulk` write cap) | small | **no**: the stores are changed in place, so a save point is a marker, not a copy |
| **B. per-block undo records, kept for J** | before each block, write what undoes it: the coin log record (the `pending.bin` body, as today), the trie keys it marks and inserts with its `replaced` flag, and the BIP30 tree's `(key, prior)` list | ESTIMATE **~1 MB/block**: 366 KB coin log (MEASURED) + ~0.6 MB of trie keys (64 B per mark or insert, about 2x the coin-log body) + a few KB for the BIP30 tree. **W ≈ 0.3 GB, J ≈ 2 GB**. MEASURED since: 1.05 MB per block at the tip, W = 302 MB (§15) | ESTIMATE about the apply cost, ~4 s/block: depth 1 ≈ 10 s, depth 6 ≈ 30 s, depth 288 ≈ 20 min, then re-prep of the new branch at the same rate | ~400 lines Rust | **chosen** |
| C. keep prep K blocks behind the tip | never see reorgs ≤ K | none | adds K × 10 min latency to every block | tiny | **no** as the only defence (user requirement); and it still needs B for reorgs deeper than K |
| D. replay from the newest save point ≤ F | restart from an older save | — | — | — | **impossible**: the stores cannot go back to a save point below their height (`RESET_v026.md`); this is why the v0.26 reset restarts from genesis |
| B′. trie variant: keep freed trie slots for J flushes, rewind = point the root back | retain old versions | ESTIMATE ~20 MB/block of replaced nodes, so ~6 GB for W | instant for the trie | medium | not needed: B's trie undo is exact, and B′ needs a flush per block; kept as a fallback if trie undo turns out slow |

**How B works**, per block h:

- **Write the record right after the block is applied** (corrected in P1, §15: the BIP30 priors and the trie's
  `replaced` flag exist only then), as `reorg/u_<h>.bin` (magic, h, block hash, prev hash, the three bodies,
  SHA-256). It is made durable at the next save point, before that save point's state file, so every record at or
  below a complete save point survives a power loss. Records above the newest save point are only ever needed in the
  process that wrote them: a crash undoes those blocks (`pending.bin`, the trie's last flush), and the restart deletes
  the records above its anchor and writes them again. `pending.bin` stays the crash journal, unchanged. After each
  block, delete `u_<h-J>.bin`.
- **Coin log:** `cl_undo_to(body, n)` exactly as the crash path uses it today; the record's body is the same bytes.
- **Trie:** undo needs only each mark's and insert's **key** (`t_rm`, `t_unmark`). New
  `TrieStore::undo_recorded(keys, replaced)` refuses when `replaced` is true (a BIP30 re-creation, impossible after
  BIP34 until height 1,983,702) and otherwise does what `undo` does without the in-memory `applied` stack. Undo is
  exact because the trie's shape depends only on its key set (`trie_store.rs` header). The undo writes new nodes
  (copy-on-write), and the flush at F makes them durable.
- **BIP30 tree:** restore each `(key, prior)` from `smt_advance_ex`'s second return value, newest block first
  (`None` means remove).
- **Then:** load `state_F.json`; write `state_F.{smt,coinlog,trie,forest,flags}` and flush the trie. The
  `libzync-prep-run.sh` restart rules then take F as the newest complete save point, unchanged.

**F above the last save point s** (the usual case: a depth-1 to depth-6 reorg). The undo records cover it the same
way, so nothing is replayed from s. **F at or below s:** the records reach back J blocks, so a save point is never a
floor.

The **follow mode** of `libzync-prep` is a long-lived loop with no `hi`. It saves every `HAZYNC_PREP_SAVE_EVERY`
blocks as today, and it does all rewinds **in-process**, so the in-memory BIP30 tree needs no reload. A restart
mid-rewind goes through `out/REWIND` (§5).

## 7. Coordinator rewind (one transaction per epoch E)

| what | above F (any row or file with hi > F) | kept? |
|---|---|---|
| `vranges` | moved to a new table `vranges_orphaned` (same columns, plus `epoch`) | **yes, as a record.** Removing them from `vranges` is **required**: libzync's `RANGE-OK` prints `range_work=0`, so the frontier's most-work rule (`_frontier_chain`) falls back to **highest hi**, and an orphaned branch proven higher would win |
| folds with lo ≤ F < hi | the same: their `out_tip` is an orphaned state | the parts at or below F stay; the fold tree grows again |
| `ranges` | rows with hi > F copied into `ranges_orphaned`, then deleted (rows are created on demand at submission) | record |
| claims in flight (`ranges` claimed, `_fold_claims`) | voided. The worker's next beat gets the existing 409 "not your claim" | — |
| `submissions` | untouched (append-only audit); one note row per epoch | yes |
| `contributors.blocks` | **proposal:** keep the credit. The contributor proved a real block we asked for; the network orphaned it. Shown as "n orphaned" next to the count. The board's `proven` count excludes orphaned blocks. **Decided: kept** (§14) | — |
| sponsorships marked `proven` over heights > F | back to `proving`, and their hold resumes | — |
| `proofs/proof_<rid>.bin` | moved to `proofs/orphaned/E/` (it **must** move first, or the new chain's proof with the same id overwrites it) | yes, served at `/api/proof/orphaned/E/<rid>` |
| spine | every accepted head is kept as `spine/heads/spine_<hi>.{bin,json}` for hi > tip − J (229 KB each: **66 MB for W, 0.46 GB for J**). If `spine.hi` > F, the newest kept head with hi ≤ F becomes `spine.bin`/`spine.json`, and the orphaned heads move to `spine/orphaned/E/` | yes |
| caches | `_frontier_invalidate()`, every `_sf` entry, `_servable_high(force=True)` | — |
| `meta` | `reorg_epoch = E` | — |

**Public API and website.** `/api/meta` gains
`reorg: {epoch, fork, depth, orphaned: [[h, hash]…], state: "rewinding"|"done", at}`. The block page for an orphaned
height shows the replacement block and, below it, "orphaned in reorg E" with that proof. The site shows a banner while
`state` is `rewinding`, and for an hour after it is `done`. Progress numbers simply drop by `depth` and then climb
again.

## 8. Proof soundness

**No proof of an orphaned block can be joined into the new chain**, and the board never accepts one after the
rewind.

- **Guest, one block:** a leaf or aggregate step asserts `header.prev == prev.tip_hash` (`prevhash_ok`,
  `main.rs:2302`, asserted at `:2325`; the same in the chain step `:1650/:1670` and in `prove_range` `:1728/:1736`).
- **Guest, joins:** `join` asserts `link_digest(left) == right.anchor_id` (`main.rs:2501`). `link_digest`
  (`main.rs:644`) is sha256d of the whole `ChainState` except `anchor_id`, so it covers `tip_hash` and the block
  forest (every block's hash). A new-chain leaf for F+2 is anchored on the new S(F+1); the orphaned S(F+1) has another
  `tip_hash`, so the digests differ and the join fails. A new-chain leaf for F+1 and the orphaned F+1 share the
  anchor S(F), but they **end** on different states, so nothing above them can join both.
- **Board:** `verify-any --check-end` requires the receipt's start to be the board's state before lo and its end the
  board's state at hi (`lz_coord.rs` `verify_any`). After step 3 those are the new chain's files, or none.
- **One thing to know:** an orphaned proof is a **valid proof of a valid chain**, just not the most-work one. Nothing
  in the guest compares branches. A consumer holding two genesis-anchored proofs should compare `cum_work` (it is in
  `ChainState`) or check the tip hash against its own node. That is how Bitcoin works too, and it needs no change. The
  board's choice of branch rests on its node, through prep's states.
- **Checkpoint floor:** the image pins 970,000 (`checkpoint.rs`). A reorg below it cannot be proved by this image at
  all. That is about 6,100 blocks below today's tip, deeper than J.

## 9. Offsite copies (`proofs-<id>/`, `spine-<id>/`)

The copy is append-only by name, so an orphaned `proof_<rid>.bin` already uploaded blocks the new chain's proof with
the same name forever, and `check` alarms on the size difference.

- **Depth delay (the margin that buys a lot):** `copy` uploads a proof only when its hi ≤ tip − 6 (tip from
  `/var/lib/hazync/node_tip`), and `spine` uploads only heads from `spine/heads/` that are at least 6 blocks deep. This
  costs no proving latency: the local copy is there at once, and only the off-box copy waits about an hour.
- **A reorg deeper than 6 (rare):** the orphaned objects stay where they are. The coordinator writes
  `proofs-<id>/reorgs/E.json` (fork, old and new hashes, orphaned names with sha256), and the new chain's files go up
  under `proofs-<id>/r<E>/` and `spine-<id>/r<E>/`. `check` accepts a size difference on a name that a marker covers.
  A restore from offsite reads the markers. The receipts describe themselves anyway (their journal commits the tip
  hash).
- Local orphans (`proofs/orphaned/E/`) are copied under `proofs-<id>/orphaned/E/`, so the record survives off-box.

## 10. Other followers

- **zAPP follower** (`hazync-lz-zapp-follow.sh`): today it dies when the spine is below its stores ("needs a fresh
  `ZAPP_ROOT`"). It becomes: run `libzync-prep-rewind <spine hi>` on its own stores (same host, same records), cut
  `history.bin` to `hi + 1` records (the record walk the script already does), and republish the three files. ⚠ It
  keeps its trie in memory, with a full snapshot per run. That cannot reach the mainnet tip in any case (the trie is
  290 GB at 632,000), so it will have to share the tip run's stores. **Decided:** it reads the tip run's stores (§14).
- **Header-chain follower** (`coordinator/deploy/headers/`): already proves its run again each time; a reorg above its
  base needs nothing. A reorg below the base it already refuses.
- **Leaf pruner:** keyed on the frontier, which drops after a rewind, so nothing changes.

## 11. Beyond W: stop, alert, recover

**The stop.** When the fetcher finds no agreement within W, it writes `blocks/REORG_TOO_DEEP` (the node's tip, the
deepest height compared, F if it is known within J) and exits with code 4. Prep sees it and also exits with 4. Both
units get `RestartPreventExitStatus=3 4` and an `alert.conf` with `OnFailure=hazync-alert@%n.service`. The exit has
to fail the unit: a process that waits for ever never fails, so `OnFailure=` never fires (memory:
`gotcha_watchdog_outlives_the_oneshot_it_guards`). The coordinator keeps serving everything at or below the newest
height the two chains could still share. Nothing above it can verify, because states above it are not removed until a
person acts.

**Runbook** (by hand, on `hazync-proof`):

1. Check the reorg is real and settled: `getchaintips` (the old branch should be `valid-fork`, the new one `active`),
   and compare against a public explorer. If our node is the odd one out, stop here: fix the node, then start the
   units again; nothing needs undoing.
2. Find F: `fetch_block_rpc.py fork-point --deep <out>` walks back without the W limit and prints F and whether
   F ≥ tip − J (the undo records still reach it).
3. **F within J** (the expected case): `libzync-prep-rewind F --allow-deep <out>`, the same code as the automatic
   path behind an override flag. It writes the record, waits for the coordinator's ack, and undoes the stores. Then
   start the units.
4. **F below J:** the stores cannot be undone. Rewind the coordinator to F with `server.py --reorg-to F` (works at any
   depth: it only touches rows and files). Then restart prep from genesis under a fresh `LZ_ROOT`, moving the old one
   aside, as the v0.26 reset does. ESTIMATE about 8 days to 611,000 (MEASURED on the old run, `RESET_v026.md`), then
   the remainder.
5. **F below the checkpoint (970,000):** no proof exists for that chain under this image. A new image with another
   checkpoint is a release decision.

## 12. Testing

| layer | what | where, cost |
|---|---|---|
| unit (Rust, CPU) | `undo_recorded` against a fresh build, across a close and reopen, on random blocks (extends the existing `trie_store` undo tests); coin log records kept across `cl_journal_commit` and undone below a save; BIP30 priors restored; a refused `replaced` | `cargo test`, seconds |
| prep round trip (CPU) | prep 1..60 regtest blocks, rewind to 40, re-prep a different 41..60: **every `state_<h>.json` byte-identical to a fresh run on the new branch** (as `HAZYNC_PREP_CHECK` does), and the leaf bundles identical | local, about a minute with the regtest host |
| crash | the existing `HAZYNC_CRASH_AT` hook gains `rewind-moved`, `rewind-after-undo:<h>` and `rewind-before-save`; each restart must finish the rewind and pass the round trip | local |
| coordinator (Python, mock) | `COORD_ALLOW_MOCK` with `RISC0_DEV_MODE` + `HAZYNC_TEST_FAKE_RECEIPTS=1` fake receipts: rows moved, frontier and spine back at F, a submission whose verification straddles the rewind gets 409 `reorged`, a voided claim's beat gets 409, ids reused cleanly, an epoch applied at start-up | `coordinator/test_reorg.py`, seconds, CPU |
| end to end (regtest) | `bitcoind -regtest`, the fetcher and prep in follow mode, the coordinator in mock mode, a fake worker. Reorgs with `invalidateblock` + `generatetoaddress` at **depth 1, 2, W, W+1**, each once **mid-prep** (between the fetch and the apply) and once **mid-prove** (a claim held, a submission verifying). Expected: 1, 2 and W handled with no person; W+1 makes the units fail with exit 4 and the alert fires (stubbed). Run with `W=12` for speed and once at 288 | local or the box, minutes |
| offsite | a moto/minio S3 stub: depth delay, marker, `check` accepting a covered name | Python, seconds |

**CI:** the unit tests and the Python mock tests are cheap (CPU, seconds). The regtest end-to-end test needs a
regtest host build, which is the long one, so I would run it locally or on the box and not add a workflow without
asking.

## 13. Phases

| phase | content | size (ESTIMATE) | ship |
|---|---|---|---|
| **P1 undo records (write only)** | `reorg/u_<h>.bin` per block, cut to J; BIP30 priors kept; nothing reads them yet | ~200 lines Rust + unit tests | **first, and before any run comes within J of the tip**: records exist only from when the host runs. Writes only, so low risk |
| **P2 detect + stop** | fetcher `follow` with the hash check; prep turns the `prep.rs:197` assert into exit 4; the coordinator's hash check at commit; unit `RestartPreventExitStatus` + `alert.conf` | ~180 lines | with P1. With P1 and P2 alone, any reorg **stops safely** and the manual recovery is P3's command, not genesis |
| **P3 prep rewind** | `undo_recorded`, `libzync-prep-rewind`, `out/REWIND`, follow mode, crash hooks | ~400 lines Rust + ~150 test script | after P1 |
| **P4 coordinator rewind** | records and ack, orphan tables, spine heads, file moves, `/api/meta.reorg`, `--reorg-to` | ~350 lines Python + ~300 test | with P3; automatic handling starts when both are in |
| **P5 followers** | offsite depth delay + markers; zAPP follower rewind; website banner (web repo) | ~200 lines | after P4 |
| **P6 regtest harness** | §12 end to end | ~300 lines | alongside P3/P4; it gates turning on automatic handling |

Each phase goes through a code review before its PR, and P3/P4 through more than one round, as the prep crash work
did.

## 14. Decisions (the user, 16.11.2026)

Each open question was decided as recommended:

1. **D1, the tip run:** the **paper prep** (`libzync-prep`, `LZ_ROOT=/srv/bulk/libzync-prep`) follows the tip. The
   coordinator will need a second bridge and state directory above some height (P4/P5).
2. **J = 2,016:** undo records are kept for 2,016 blocks, on top of automatic handling within W = 288. A fork deeper
   than W but within J is a one-command recovery (P3's `libzync-prep-rewind --allow-deep`).
3. **Contributor credit for orphaned proofs:** kept, shown as "n orphaned" next to the count. The board's `proven`
   count excludes orphaned blocks.
4. **Offsite depth delay:** 6 blocks. Offsite copies upload only proofs (and spine heads) at least 6 blocks deep.
5. **Claims margin:** 0. Tip blocks are claimed at once (`TIP_CLAIM_DEPTH` default 0).
6. **zAPP facts at the tip:** read from the tip run's stores (history mode on the paper prep), not the follower's own
   in-memory trie.
7. **Forks deeper than J:** accept a genesis restart. No periodic cold copies of the stores.

## 15. Built: P1 and P2 (16.11.2026)

**P1, undo records** (`prover/host/src/reorg.rs`, `prep.rs`, `lz.rs`; `libzync/src/trie_store.rs` gains one accessor,
`last_apply_replaced`, host only). Every block prep applies leaves `<out>/reorg/u_<h>.bin`: magic `LZUNDO01`, height,
block hash, prev hash, the coin log body (byte for byte `pending.bin`'s record body, `cl_undo_body`), the trie's
`replaced` flag and the KEYS of its marks and inserts, the BIP30 tree's `(key, prior)` list, SHA-256. Kept for J =
`HAZYNC_PREP_UNDO_KEEP` blocks (default 2,016 with on-disk stores, 0 = none; in-memory runs default to 0). Nothing reads
them yet. Tests show each part is enough to undo its block: the coin log body undoes random blocks on a disk store
back to any earlier block (`lz.rs`), the trie keys undo random blocks in the session (`reorg.rs`), the BIP30 priors
undo inserts and tracked spends (`reorg.rs`); on mainnet 1..200 the records match the states, are byte-identical after a
crash and restart, and leave every state byte-identical to a run without them
(`experimental/libzync-bench/prep/reorg_test.sh`).

**P2, detect and stop** (`prover/fetch_block_rpc.py`, `prep.rs`, the four prep and fetch units, four `alert.conf`
drop-ins). With `--after-saves`, `range` (and the new `follow`) check prep's newest `state_<h>.json` against the node
every poll (5 s) and, when the node's best block changes, every waiting block file (its header hashed from the file).
**P2 stops for every fork below prep's tip**: `blocks/REORG` with `class` `auto` (depth ≤ W, P3 will handle it with no
person) or `manual` (W < depth ≤ J), or `blocks/REORG_TOO_DEEP` (no agreement within J), and exit 4. A fork above
prep's tip only replaces the waiting files. Prep exits 4 while a marker is there, and in place of the `prep.rs:197`
assert it waits for the fetcher (the file replaced, or a marker) up to `HAZYNC_PREP_WAIT_S`, then exits 4. The units:
`RestartPreventExitStatus=3 4` (prep) and `4` (fetch), `OnFailure=hazync-alert@%n.service`. `fetch_block_rpc.py
fork-point <out> [--deep]` is the runbook's step 2. **P2 recovery, until P3:** a person checks the reorg (§11 step 1),
recovers the stores, then removes the marker from the block directory and starts both units.

**Record size, MEASURED** from the blocks (the node, read-only RPC; each block's counts put through the record's exact
layout: external spends, created and appended coins, as prep counts them), not from a prep run at those heights:

| blocks | mean | median | max | coin log part (mean) |
|---|---|---|---|---|
| 632,001..632,587 (the 587 blocks of §2's `pending.bin`) | 1.04 MB | 1.10 MB | 1.71 MB | 365 KB (`pending.bin`: 366 KB, §2) |
| 975,818..976,105 (the last 288 at the time) | 1.05 MB | 0.98 MB | 1.95 MB | 292 KB |

So W = 288 blocks at the tip is **302 MB (MEASURED)** and J = 2,016 about **2.1 GB** (ESTIMATE, 2,016 x the tip mean).
The trie keys are about two thirds of a record at the tip (64 bytes for each of ~4,100 spends and ~7,700 new coins).

**Image id, MEASURED.** No guest input changed (`git diff` against `797b777e`: nothing under `prover/methods/`, none
of the `#[path]` files, no manifest, lock or toolchain file; `scripts/check-guest-inputs.sh` passes). Built from source
on `hazync-coord` as `reproduce/libzync-ids.sh` builds it (rustc 1.99.1 from `prover/rust-toolchain.toml`, the base
from `scripts/libzync-base.sh`), with the container's paths (the checkout bind-mounted at `/hazync-zkvm` and the base
at `/root/hazync-libzync-base`, in a private mount namespace): `host method-id` = **`46666b6f…43c1c`**, guest ELF
sha256 **`f7b87bf1…08a27`**, both exactly `reproduce/LIBZYNC_IDS`. ⚠ The same build from another checkout path gives
another id (`c4556aaf` for this branch, `b021c7f3` for `797b777e` itself, each from its own path): the ELF embeds one
absolute path, its build's `OUT_DIR` (`…/out/core_extract.inc`); the two ELFs differed in that string only.

**What building it corrected in this page:**

- **§6 "write the record before the stores change"** cannot hold for the BIP30 priors and the trie's `replaced` flag,
  which exist only after the block is applied, and is not needed (see §6 as corrected). What is needed and was missing:
  the restart must **delete the records above its anchor**, or P3 would read records of blocks a crash undid.
- **§4 and §11 disagreed** on when `REORG_TOO_DEEP` is written (§4: no agreement within J; §11: within W). Built: J.
  Depth ≤ W or ≤ J is the `class` in `REORG`.
- **A node behind prep** (still syncing, or a reorg to a shorter chain) was not covered: comparing prep's tip with a
  height the node does not have would read as a reorg. Built: compare at the node's height; on prep's chain, wait.
- **§11 "nothing above [the fork] can verify"** is wrong while only P1 and P2 are in: no state file moves, so
  `verify-any --check-end` still accepts proofs of the **orphaned** blocks up to prep's tip, and the frontier can grow on
  them, until a person acts. Sound (they cannot join the new chain, §8), but the board can credit work on the orphaned
  branch for that time. Only P3/P4 (moving the states) close it.
- **The coordinator's hash check at commit** (listed in P2) can never fire while no state file moves: it belongs with
  P3/P4 and was left to them.
- **Where fetching starts** must stay prep's newest complete save point + 1, never prep's newest state file + 1: state
  files above the save point (an older run's, or a crashed run's) would move the start past the blocks prep needs.
- **Waiting block files carry no hash.** The fetcher hashes each file's header (the first 4 KB hold the header fields
  in every writer's key order; else the whole file), checked against the 200 mainnet test blocks' hashes.
