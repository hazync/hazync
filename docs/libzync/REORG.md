# Reorg handling for the libzync board (design)

> **Design, 16.11.2026; P1 and P2 built the same day** (branch `feat/reorg-p1-p2`, §15), P3 too (§16), P4 and P6 on
> 18.11.2026 (§17), P5 the same day (§18); **D1, the tip run, on 20.11.2026 (§19)**. I wrote it from the repo at `797b777e` (main) and a read-only survey of `hazync-proof` the same day.
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
| the tip run | **decided:** the paper prep (`libzync-prep`, `LZ_ROOT=/srv/bulk/libzync-prep`) follows the tip (§3); **built** (§19): the board serves blocks from X = 974,937 from it |
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
tip run" is whichever one it is. **Decided (16.11.2026): the paper prep is the tip run** (§14). **Built (20.11.2026,
§19):** the handover height X = 974,937 (the board pass's `LZ_HI` + 1).

## 4. Detection

**One comparison catches a reorg of any depth.** If the node's active chain has prep's tip hash at prep's tip height,
then every block below it is on the active chain too, because headers chain by hash. If it does not, there was a
reorg at or below prep's tip, however deep.

| stage | how it learns | what it does |
|---|---|---|
| **fetcher** (`fetch_block_rpc.py follow`, the only stage with RPC) | each poll (every 5 s): `getbestblockhash`. When that changes, it walks back by `previousblockhash` from the node's tip until it meets a height ≤ prep's tip whose hash equals prep's `state_<h>.json` `tip_hash` (the **fork F**). It also checks each waiting `block_<h>.json` against `getblockhash(h)` | fork above prep's tip: it deletes and refetches the waiting files above F (nothing else changes). Fork below prep's tip: it writes `blocks/REORG` (F, old and new hashes F+1..) and stops fetching. No agreement within J: it writes `blocks/REORG_TOO_DEEP` (§11) |
| **prep** (follow mode) | (since round 3) before each block, a `state_<h>.json` an earlier run left for ANOTHER block is a fork at h-1: prep writes `blocks/REORG` itself (`source: prep`, §17) and stops, as below. Between blocks, it checks for `blocks/REORG` (it already polls that directory every 200 ms). It also replaces the `prep.rs:197` assert with a check: a block whose `prev` is not its tip means "reorg suspected", so it waits for the fetcher's verdict instead of panicking | runs the protocol in §5. Exits with code **4** when the fork is deeper than W |
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
  the records above its anchor and writes them again. `pending.bin` stays the crash journal, unchanged. The window is
  cut **only at save points** (at save s, every record ≤ s − J goes; review, M3): cut per block, a crash at c and a
  restart from the save point A left only J − (c − A) records below A. So at every moment the records reach at least J
  blocks below prep's position, and below every save point a restart can take; no relation between
  `HAZYNC_PREP_SAVE_EVERY` and J or W is needed (between save points the window holds up to J + SAVE_EVERY records).
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
| sponsorships marked `proven` over heights > F | back to `proving`, and their hold resumes | — (⚠ below) |
| `proofs/proof_<rid>.bin` | moved to `proofs/orphaned/E/` (it **must** move first, or the new chain's proof with the same id overwrites it) | yes, served at `/api/proof/orphaned/E/<rid>` |
| spine | every accepted head is kept as `spine/heads/spine_<hi>.{bin,json}` for hi > tip − J (229 KB each: **66 MB for W, 0.46 GB for J**). If `spine.hi` > F, the newest kept head with hi ≤ F becomes `spine.bin`/`spine.json`, and the orphaned heads move to `spine/orphaned/E/` | yes |
| caches | `_frontier_invalidate()`, every `_sf` entry, `_servable_high(force=True)` | — |
| `meta` | `reorg_epoch = E` | — |

⚠ **A reopened sponsorship holds the new branch's blocks for the sponsor bot ALONE** (review of P4, L8). Normal
workers are kept off them (`SPONSOR_HOLD_SQL`), so at the tip the frontier waits for the bot's next timer run and a pod
rental; the only signal is the hold alert after `SPONSOR_HOLD_ALERT` (6 h: `/api/state` `attention`, the board
progress check). If the bot is down (its breaker latched, `hazync-check-breakers` reports it): start it again
(`systemctl start hazync-sponsor-bot.service` after the cause is fixed); if it cannot run soon, release the hold by
hand, so any worker proves the blocks, as the coordinator's user:

```bash
sudo -u hazync python3 -c "import sqlite3,time; c=sqlite3.connect('/var/lib/hazync-lz/coordinator.db'); \
  c.execute(\"UPDATE sponsorships SET status='proven', proven_at=?, note=COALESCE(note,'')||' [hold released by hand after reorg]' \
  WHERE id=? AND status='proving'\", (time.time(), <id>)); c.commit()"
```

(`proven` is what the sponsorship becomes anyway once its blocks are proven; it shows as proven a little early.)

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
   F ≥ tip − J (the undo records still reach it), and any marker already in the block directory. A marker with
   `source: prep` is prep's own: its state files above its save point were another chain's (a reorg inside its replay
   window, §17); its `fork` is right, and step 3 applies.
3. **F within J** (the expected case), as built in P3: with both units stopped and the fetcher's `blocks/REORG` naming
   F (edit `fork` if `fork-point --deep` found it),

   ```bash
   cd "$LZ_ROOT" && HAZYNC_COINLOG_DIR=$LZ_ROOT/cl HAZYNC_TRIE_DIR=$LZ_ROOT/trie HAZYNC_PREP_LEAF_BUNDLES=$LZ_ROOT/leaves \
     HAZYNC_PREP_UNDO_KEEP=<J of the run> HAZYNC_REORG_ACK_DIR=<the unit's, e.g. /var/lib/hazync-lz/reorg-acks> \
     bin/host_lz libzync-prep-rewind $LZ_ROOT/out $LZ_ROOT/blocks --allow-deep
   ```

   It refuses (exit 4, nothing changed) without the three directories (`--no-leaf-bundles` for a run that writes no
   bundles), or when the records do not reach F. It undoes the stores to F, writes a save point there, moves everything
   above F to `out/orphaned/<epoch>/` (the bundles to `leaves/orphaned/<epoch>/`) and removes the marker. Then start
   the units. Since P4 (§17) it also writes the record and waits for the board coordinator's ack (the coordinator must
   be running); a run no coordinator follows takes `HAZYNC_REORG_ACK=0`.
4. **F below J:** the stores cannot be undone. Rewind the coordinator to F with `server.py --reorg-to F` (works at any
   depth: it only touches rows and files). With the coordinator stopped, as its user and with its unit's environment
   (it refuses without `COORD_DB`, as root, or while the port is in use; without `--yes` it only says what would move):

   ```bash
   systemctl stop hazync-lz-coordinator
   sudo -u hazync env $(systemctl show -p Environment --value hazync-lz-coordinator) \
     python3 /opt/hazync-lz/coordinator/server.py --reorg-to F          # then again with --yes
   systemctl start hazync-lz-coordinator
   ```

   (`systemctl show -p Environment` joins every drop-in's settings; none of the unit's values contain spaces.) Then restart prep from genesis under a fresh `LZ_ROOT`, moving the old one
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
| **P4 coordinator rewind** | records and ack, orphan tables, spine heads, file moves, `/api/meta.reorg`, `--reorg-to` | ~350 lines Python + ~300 test | with P3; automatic handling at the tip needs P5 too (§17, review M4) |
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
back to any earlier block (`lz.rs`), the trie keys undo random blocks in the session (`reorg.rs`) and, through the new
`TrieStore::undo_recorded` (host only, `std`; review M5), blocks applied in an EARLIER session: a disk trie reopened
from its last flush, each block undone from its keys alone, equals snapshot byte for byte the trie of the blocks below
it, and stays equal after a flush and a reopen (`trie_store.rs`). P3 builds on that function. The BIP30 priors
undo inserts and tracked spends (`reorg.rs`); on mainnet 1..200 the records match the states, are byte-identical after a
crash and restart, and leave every state byte-identical to a run without them
(`experimental/libzync-bench/prep/reorg_test.sh`).

**P2, detect and stop** (`prover/fetch_block_rpc.py`, `prep.rs`, the four prep and fetch units, four `alert.conf`
drop-ins). With `--after-saves`, `range` (and the new `follow`) check **prep's tip** against the node every poll (5 s),
every waiting block file when the node's best block changes (its header hashed from the file), and every waiting file
that is new since the last poll. Prep's tip is `<out>/PREP_TIP.json`, which prep writes at its start and after every
block (atomically; synced at save points): the height its STORES are at, that block's hash, J, and `undo_from`, the
lowest height whose undo record exists. Not the highest `state_<h>.json`: a crashed run leaves state files above the save
point its restart takes (review, M2). A host from before the file is read from its state files, down to its newest
complete save point; state files that exist but do not read stop the fetcher ("cannot check", exit 4), never read as
"no reorg" (M1), and so does a missing or unreadable state inside the fork walk (M4). Depth is counted from prep's tip
(L1); J and the records' reach come from the tip file (L2): a fork the records cannot undo (deeper than J, or below
`undo_from`, e.g. older than the upgrade to this host) is `REORG_TOO_DEEP`. Prep's state files between save points are
written by rename and synced at the next save point (M4; the bytes are unchanged), and a block file that is there but
does not read is waited for in consume mode (L3).
**P2 stops for every fork below prep's tip**: `blocks/REORG` with `class` `auto` (depth ≤ W, P3 will handle it with no
person) or `manual` (W < depth ≤ J), or `blocks/REORG_TOO_DEEP` (no agreement within J), and exit 4. A fork above
prep's tip only replaces the waiting files. Prep exits 4 while a marker is there, and in place of the `prep.rs:197`
assert it waits for the fetcher (the file replaced, or a marker) up to `HAZYNC_PREP_WAIT_S`, then exits 4. The units:
`RestartPreventExitStatus=3 4` (prep) and `4` (fetch), `OnFailure=hazync-alert@%n.service`. `fetch_block_rpc.py
fork-point <out> [--deep]` is the runbook's step 2. **P2 recovery, until P3:** a person checks the reorg (§11 step 1)
and recovers the stores (until P3's `libzync-prep-rewind`, that is §11 step 4: prep from genesis under a fresh
`LZ_ROOT`); in a kept `out/`, moves aside every `state_<h>.json` (and its markers) ABOVE the stores' height in
`PREP_TIP.json`: they are the orphaned branch's, the coordinator serves them, and the fetcher compares them. Then the
marker in the block directory goes, and both units start. **Install:** the unit files AND the four `alert.conf`
drop-ins (`coordinator/deploy/RUNBOOK.md`, alerting); `scripts/check-unit-drift.sh` now checks the `libzync-*` units
too and reports a running or enabled unit without a drop-in the repo declares for it (M6). ⚠ On `hazync-proof` its
first run will also report `libzync-prep`'s `run.conf` and `cache.conf` and `libzync-fetch`'s `run.conf`, drop-ins
that exist only on the box (keys `LZ_ROOT`, `LZ_HI`, `HAZYNC_PREP_*`, `HAZYNC_TRIE_CACHE_MB`, `HAZYNC_RPC_COOKIE`):
commit them or allow-list their keys.

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

## 16. Built: P3, prep's rewind (16.11.2026)

Branch `feat/reorg-p3` (on top of P1, P2, the review fixes and ZAPP_FOLLOWER Z1).

**`host libzync-prep-rewind <out> <block dir> [--allow-deep]`** (`prover/host/src/rewind.rs`). The target is `out/REWIND`
(an interrupted rewind) or the fetcher's `blocks/REORG`: class `auto` (depth ≤ W), or any fork within J with
`--allow-deep` (§11 step 3). Everything is checked before anything changes, and a refusal (exit 4) leaves nothing
behind: REORG_TOO_DEEP, depth beyond W without the flag, a missing or damaged record, records that do not chain from
F's block to A's, a block that re-created a coin (`replaced`). Then `out/REWIND` (fork, epoch, the marker) is written
durably, and from the newest complete save point A:

1. the trie: opened (which bumps `trie/GEN`, Z1), its logged roots above F cut (kept as `out/orphaned/<E>/roots.log`),
   each block undone from its record's keys newest first (`TrieStore::undo_recorded`), the root checked against
   `state_F.json`'s `trie_root`; the disk is unchanged until the save point's flush;
2. the coin log, in place (`lz::cl_rewind`): `pending.bin` (blocks after A) and then the records' bodies, newest first,
   each by `cl_undo_to` to F's leaf count; roots and leaf count checked against `state_F.json`;
3. the BIP30 tree from `state_A.smt`, each record's priors restored newest first, the root checked;
4. a complete save point at F (`prep::save_stores`, the same code as prep's: markers, `.smt`, `.coinlog`, `.trie`,
   the journal committed); `state_F.json` keeps its bytes;
5. everything above F moved to `out/orphaned/<E>/`: every `state_<h>.*` (the save files first, so A stops being a
   complete save point before its state goes), the leaf bundles (`leaves/`); the records above F deleted, and the block
   files whose block is an orphaned one;
6. `PREP_TIP.json` at F, then `out/REWIND` and the marker removed, last.

Every step is idempotent, so a rewind that dies is run again from `out/REWIND`. If A ≤ F (prep stopped between save
points with F above its last one) nothing is undone: the files above F move and prep resumes from A as after any
crash. **The automatic path** (`HAZYNC_REORG_AUTO=1`, off by default until P4): the fetcher writes `REORG` class `auto`
and waits instead of exiting; prep exits **5** (`PREP_STOPPED_FOR_REWIND`; not in `RestartPreventExitStatus`, so systemd
restarts it); `libzync-prep-run.sh` runs the rewind before it picks the save point; the fetcher carries on after the
marker goes. `host libzync-store-digest <out> <h>` prints the CONTENT of the stores (trie snapshot, coin log leaves,
index entries, BIP30 snapshot) for comparing runs reached by different histories.

**Where this differs from §5-§6.** (a) The rewind runs at a (re)start from the newest save point, not in-process in a
follow-mode prep: one code path for the normal case and for a crash, at the cost of one restart (`RestartSec=30`) and
replaying from A. (b) No coordinator steps (records, ack, §5 steps 4-6): that is P4, which is why automatic rewinds stay
off. (c) Prep's follow mode (no static `LZ_HI`) is not built. (d) Of §12's crash hooks only `rewind-after-save` is
built; the steps before it change only the coin log in place (idempotent) and nothing on disk else.

**Tests.** `experimental/libzync-bench/prep/rewind_test.sh`, mainnet 1..200 (test data) as the old chain and a synthetic
branch from F (`reorg_branch.py`: real header hashes, a coinbase per block, a spend of a shared-chain coinbase and an
in-block spend) as the new one, W = 12, J = 60, saves every 50, through the deployed runner. After the rewind and the
re-prep, every `state_<h>.json`, the save point's files, the leaf bundles, the undo records, `PREP_TIP.json`, the
logged roots (height, hash, root hash) and the stores' CONTENT equal a FRESH prep of the new branch: depth 1, 2 and W
from a save point at 200; depth 1 and W with prep stopped between save points (journal of 151..200); the automatic
path end to end (exit 5, restart, rewind); a crash after the save point at F, finished from `out/REWIND`; depth W+1
with `--allow-deep`; and three refusals that change nothing. `--control` changes one byte of each rewound coin log after the rewind: exactly the three comparisons must fail, each
on the coin log's content (the rewinds themselves pass).
`prover/test_fetch_reorg.py`: the fetcher waits for an `auto` rewind and fetches the new branch after it; a fork beyond
W still stops. `coordinator/test_reorg_units.py`: exit 5 restarts the prep units and does not alert.

**Orphaned files are kept** (`out/orphaned/<epoch>/`, `leaves/orphaned/<epoch>/`) until a person removes them; nothing
prunes them. Keep at least the epochs whose blocks are within J of the tip (P4 copies them off-box first). The epoch's
directory is made with the intent, so epochs are unique even when a rewind moves nothing.

**Review of 16.11.2026 (P3), fixed.** (H1) `cl_rewind` undid every `pending.bin` record, also those of blocks a save
point already held (a crash between the trie's flush at A and the journal's commit leaves them): now only records above
F, and a record without its height is refused. (M1) The trie and the BIP30 tree are undone in memory and checked
before the coin log, the one store changed in place, is touched; after the intent any failure exits 4 (the unit fails
and alerts) instead of a panic's 101 that systemd restarts in silence; leaf bundles move within their own filesystem
(`leaves/orphaned/<epoch>/`); the rewind refuses to run without `HAZYNC_COINLOG_DIR`, `HAZYNC_TRIE_DIR` and
`HAZYNC_PREP_LEAF_BUNDLES` (or `--no-leaf-bundles`). (M2) A fetcher started while an `auto` marker is there finds where
to fetch again after the rewind; it waits at most `HAZYNC_REWIND_WAIT_S` (3600) and then exits 4. (M3) The orphaned
blocks (height, hash) are recorded in `out/REWIND`, so a rerun after a crash during the moves still finds every
orphaned block file. (M4, Z1) The logged roots above F are archived before they are cut, and F's new root is logged in
the new generation. Prep started with an `auto` marker exits 5 (for the runner) instead of 4.

**Merging P3.** `feat/reorg-p3` holds one commit that does not build (`fbb6e8f6`, a merge committed with conflict
markers, repaired by the next commit `69a5d934`): squash-merge the branch, and say so in the PR text.

## 17. Built: P4, the coordinator's rewind, and P6, the end-to-end harness (18.11.2026)

Branch `feat/reorg-p4` (on top of `feat/reorg-p3`). Host only on the prep side: no guest input changed.

**Prep's side** (`prover/host/src/rewind.rs`, `record_and_ack`). Once everything above F has moved (§16 step 5), the
rewind writes `out/reorg/records/<E>.json` once, durably: `epoch`, `fork`, `fork_hash`, `depth`, `orphaned` and `new`
as `[height, hash]` (display hex), `at`. Then it waits for `<E>.ack` carrying the record's sha256, and only then moves
prep's position to F and removes `out/REWIND` and the fetcher's marker, so nothing of the new branch is published while
the board still holds the orphaned rows (§5 step 6). The ack is read from `HAZYNC_REORG_ACK_DIR` (default: the records'
directory). On `hazync-proof` prep runs as root and the coordinator as `hazync`, so the coordinator cannot write prep's
`out/`: both units now name `/var/lib/hazync-lz/reorg-acks` (`libzync-board-prep.service` `HAZYNC_REORG_ACK_DIR`,
`hazync-lz-coordinator.service` `HAZYNC_LZ_REORG_ACK_DIR`). No ack within `HAZYNC_REORG_ACK_TIMEOUT_S` (600, the
`REORG_ACK_TIMEOUT` of §5): exit 4, the unit fails and alerts, the intent stays and a rerun waits again (§5 said "alerts
and keeps waiting"; a process that waits never fails its unit, so `OnFailure=` would never fire). `HAZYNC_REORG_ACK=0`:
no coordinator follows the run, nothing is waited for (the record is still written). Crash hook `rewind-after-record`.

**The coordinator** (`coordinator/server.py`, the "reorgs" section). A thread polls the records (`$HAZYNC_LZ_STATE_DIR/
reorg/records`, or `HAZYNC_LZ_REORG_DIR`) every `REORG_POLL_S` (2 s), and once at start-up **before the port is bound**
(a failure there stops the start). A record is applied (`reorg_rewind`) in one transaction under `_lock`:

| what | done |
|---|---|
| `vranges`, `ranges` with hi > F | copied to `vranges_orphaned` / `ranges_orphaned` (same columns + `epoch`), then deleted. Claims above F are voided with them (a beat gets the existing 409 "not your claim"); in-memory fold claims above F dropped |
| proofs | `proofs/proof_<rid>.bin` of every moved row to `proofs/orphaned/<E>/`, BEFORE the commit; served at `/api/proof/orphaned/<E>/<rid>` |
| spine | every accepted head is kept as `spine/heads/spine_<hi>.{bin,json}` (`SPINE_HEADS_KEEP` = 2,016 blocks below the head; the current one is seeded at start-up). A head above F falls back to the newest kept head at or below F; the heads above F (and the current one) move to `spine/orphaned/<E>/`. No kept head at or below F: no spine until `hazync spine` rebuilds one. The head's json now carries `out_bhash` (ZAPP_FOLLOWER §7) |
| sponsorships | `proven` with hi > F back to `proving` (their hold resumes) |
| audit | `submissions` untouched but for one row `reorg:<E>:spine-<hi>`; the spine walks (`spine_segments`, `spine_absorption`) restart from that head |
| `contributors` | credit kept (§14 decision 3): `proved` still counts the orphaned blocks, `orphaned` says how many (per contributor, `/api/provers`); the board's `proven` excludes them |
| caches, meta | every single-flight entry, `_servable_high(force=True)`; `reorgs` table row, `meta.reorg_epoch` |

Then `<E>.ack` = `{epoch, record_sha256, coord_epoch, fork, at}`. A record is known by its sha256: applied once,
acked again if its ack goes missing. **The coordinator's epoch** is `max(last + 1, prep's epoch)`: equal to prep's
while they move in step, unique when they do not (a manual `--reorg-to`, or prep restarted under a fresh `LZ_ROOT`).
**A record whose orphaned state F+1 is still in place is not applied** (prep has not moved its files; §5 step 3), so
the ledger never drops rows while the files that would re-verify them are still served.

**The check at commit (§4)** in `submit` and `submit_spine`, under `_lock`, after the verification that ran outside
it: 409 `reorged` (`{"reorged": true}`) when a rewind with its fork below hi was applied during the verification, or
when the receipt's `in_bhash` / `out_bhash` are not prep's `state_<lo-1>` / `state_<hi>` block NOW (a missing state
refuses: fail closed). Only with a state directory and a real verifier (the pure mock has no boundary hashes). **A
failed verification that a reorg explains** (verify-any's anchor / end / no-state messages, on a range above a fork
applied within `REORG_FAILURE_GRACE`, default `CLAIM_MAX` = 24 h) is answered 409 `reorged` and **not counted**
against the block: otherwise the workers finishing their orphaned claims would park the new chain's block after
`MAX_ATTEMPTS`.

**API.** `/api/meta.reorg` = `{epoch, fork, depth, orphaned: {count, first, last}, state: "rewinding"|"done", at,
done_at, source, detail: "/api/reorg/<E>"}` (null: never; cached 5 s, the list capped: /api/meta is every worker's
pre-flight), and `/api/meta.reorg_alert` (below). `/api/reorg/<E>` has the whole record: every orphaned block, the
node's new ones, what moved; `state` turns `done` once prep's state F+1 is a block of the new branch. `/api/block/<n>.orphaned`
lists the orphaned proofs of that height (epoch, prover, the orphaned block's hash, the proof's link).

**`server.py --reorg-to F [--yes]`** (§11 step 4): the same rewind by hand, at any depth. Without `--yes` it only says
what would move; with it, it refuses while the coordinator's port is in use (stop the unit first: its in-memory claims
and caches would answer for rows that moved).

**Tests.** `coordinator/test_reorg.py` (CI, seconds), with a fake `verify-any` that checks a receipt's chain against
the state files as `lz_coord.rs` does: rows, proofs, spine heads, sponsorship, credit, ack, a record not applied while
its states are unmoved, a submission and a spine head verified across a rewind (409 `reorged`, not credited), a voided
claim's beat (409), an orphaned proof after the rewind (409, not counted), the range id reused by the new branch, a
record applied at start-up before the port answers (a real server process), and `--reorg-to` (dry run, then `--yes`).
`--control` (no check at commit) and `--control-keep` (rows left in vranges) must fail, and do.

**P6, the harness** (`experimental/libzync-bench/prep/reorg_e2e.py`, `H=<host>`): the fetcher (`follow`, AUTO), prep
through the deployed runner restarted on exit 5 as systemd does, and the coordinator in its own process, with a fake
node (P3's `reorg_fake_node.py`: real block files, switched the way `invalidateblock` + `generatetoaddress` would), a
worker over HTTP (claim, beat, submit) and a stand-in verifier (`reorg_fake_verify.py`). Depth 1, 2 and W, each mid-prove
(the worker holding a claim and a receipt of the tip) and mid-prep (blocks fetched and waiting, prep stopped by SIGSTOP
between them, continued once the fetcher's marker is there), then W+1: both stop with exit 4 and nothing moves on the
board, then the runbook's `--allow-deep` with the coordinator's ack. After each: prep at the node's tip, the record
applied and acked, every verified range on prep's chain, the frontier at the new tip, nothing parked, the held
receipt refused 409 `reorged` and not counted. `--control` (the coordinator ignores the records, prep waits for no ack)
must fail. `reorg_branch.py` spends a branch's own coinbases once the shared ones run out, for branches longer than
~95 blocks (W = 288).

**Results, 18.11.2026** (laptop, a fasthost build of this branch embedding the published guest: `host method-id` =
`46666b6f…`; the guest ELF taken out of the v0.26.0 release host by its pinned sha256 `f7b87bf1…`):
`reorg_e2e.py --w 12` **PASS** (45 checks: depth 1, 2, 12 mid-prove and mid-prep, prep stopped at 68 of 85, 96 of 112
and 133 of 141 fetched blocks; 13 stopped with exit 4, then `--allow-deep`; 7 epochs; nothing parked); `--w 288` **PASS**
(47 checks; the depth-288 mid-prep reorg forked at 429 with prep stopped at 717 of 725); `--control` fails 8 checks, as
it must (the board stays on the orphaned block 63 and never reaches the new tip). `rewind_test.sh` (P3) passes all 53
checks with this host (with `HAZYNC_REORG_ACK=0`). `coordinator/test_reorg.py` PASS and both controls fail; the 53
other coordinator tests that load `server.py` pass.

**Found by the harness, fixed:** a reorg-shaped verification failure was counted against the NEW chain's block when
`verify-any`'s words fell outside the coordinator's 160-character note (lz_coord.rs puts two 64-hex hashes after
them): depth 1 has the shared anchor, so only the end check fails, and its message is the long one. The words are now
kept at the front of the note. **Seen and accepted:** a worker that submits an orphaned block above what the board
serves again (its bundle moved) gets 400 "invalid range id" rather than 409 `reorged`: refused before anything is
recorded; once the new branch serves that height, the same receipt gets 409 `reorged`, not counted.

**Where this differs from the design.**

- **Not `bitcoind -regtest`.** Prep under a mainnet host cannot prep regtest blocks, and a regtest host's guest is built
  from source (the long build). The fake node serves the same RPC calls from real files (as P3's case F). The guest is
  not exercised at all: the synthetic branches have no valid scripts or merkle roots, so no receipt of them can exist,
  not even a dev-mode one; `reorg_fake_verify.py` stands in for `verify-any --check-end`.
- **Order of §5.** P3's rewind undoes the stores BEFORE it moves the files (§16 (a)); the record and the ack come after
  the moves, before the position, the intent and the marker. Nothing of the new branch can be published before the
  ack, which is what §5's order protects.
- **Ack timeout** exits 4 instead of "alerts and keeps waiting" (above).
- **The ack directory** is configurable on both sides (prep's `out/` is root's).
- **The tip run.** The coordinator followed ONE prep run, through `HAZYNC_LZ_STATE_DIR`: the board pass. §14 D1 (the
  paper prep follows the tip; a second bridge and state directory above some height) was built on 20.11.2026: §19.

**Review of 18.11.2026 (Fable, `552b443`), fixed.** (H1) Prep's epoch was the highest `out/orphaned/<E>` + 1, and an
existing `records/<E>.json` was kept unchecked: with `out/orphaned/` cleared by hand, the next rewind reused epoch 1,
whose old ack named the old record, so prep went on while the coordinator never rewound. The epoch is now above every
orphaned directory AND every record, and a record already there that is not this rewind's (fork, its block, the
orphaned list) is exit 4. Backstop: each poll compares the board's highest verified range with prep's state at its
height; a mismatch is `/api/meta.reorg_alert` and one journal line. (M1) A record is applied only while prep's
`out/REWIND` names its epoch (prep keeps its intent until the ack), and only within `REORG_AUTO_MAX_DEPTH` (2,016) of
the highest verified block; otherwise it is refused loudly and not acked (a record a run with `HAZYNC_REORG_ACK=0`
left behind, the paper prep today, can no longer be applied at the first start after D1; deeper is `--reorg-to` only).
(M2) The spine fall-back writes the kept head's bytes before its json, and a rerun with no `spine.json` restores the
kept head (a crash between them used to leave the orphaned bytes served). (M3) Pruning kept heads always keeps the
newest one below the window (heads 10 and 3,000 kept 3,000 alone). Low: `--reorg-to` (L1, above); a cache value
computed before a rewind is not stored after it (L2: a generation counter); `/api/meta.reorg` capped (L3); feed rows
carry `kind` (`range`, `spine`, `reorg`; L4: the site still has to render `reorg`, P5/web); a marker left after a
finished rewind (prep already at the fork's block) is removed without opening an empty epoch (L5); a coordinator with
`PEER_COORDINATORS` set refuses to start with reorg handling (L6: peer sync could adopt orphaned rows back); a record
that does not read stops the start (L7). Tests: `test_reorg.py` gains a control per fix (`--control-m1`, `-m2`, `-m3`,
`-h1`), each failing without it; `rewind_test.sh` case K (H1, another rewind's record, L5); `reorg_e2e.py` gains a
rewind killed at `rewind-after-record` and restarted, a spine case, systemd's restart rule, and a control that flips one
thing (the coordinator reads no records). **Re-run after the fixes:** `reorg_e2e.py --w 12` PASS (49 checks: the killed
rewind exited 9 and was finished by the restart, applied once; the spine fell back from 116 to the kept head 103 and
advanced on the new branch), `--w 288` PASS (51), the control fails 15; `rewind_test.sh` 57/57 (case K fails 3 with the
pre-fix host); `test_reorg.py` PASS and its six controls fail; the other coordinator tests pass.

**The cost of exit 4 on an ack timeout** (review of P4, L9). A coordinator that is down, or not applying records, for
more than `HAZYNC_REORG_ACK_TIMEOUT_S` (600 s) during a reorg turns the automatic rewind into a manual restart: prep
exits 4 with `out/REWIND` kept (the unit fails and alerts), and the fetcher, waiting for the marker to go, exits 4 after
`HAZYNC_REWIND_WAIT_S` (3,600 s). Nothing is lost: the rewind is idempotent and resumes from the intent. Recovery, once
the coordinator is serving again (`curl -s http://127.0.0.1:8899/api/meta`) and its journal shows no `[reorg] REFUSED`:

```bash
systemctl reset-failed libzync-board-prep libzync-board-fetch
systemctl start libzync-board-prep      # the runner sees out/REWIND, reruns the rewind, waits for the ack again
systemctl start libzync-board-fetch     # only if it has exited (after its 3,600 s); it resumes after the rewind
```

**Automatic handling stays OFF** (`HAZYNC_REORG_AUTO` unset in every unit), and **must not go on at the tip before P5**
(review M4): offsite copies are append-only by name with no depth delay (§9), so an orphaned proof uploaded blocks the
new chain's proof of the same id for good; and the zAPP follower dies when the spine drops below its stores (§10). §13's
"P3 and P4 in, gated by P6" is not enough at the tip; a pass far below the tip (the board pass today, J below it) is
not affected. Once P5 is in, turning it on is a deployment decision after the reviews: the board pass and its fetcher get
`HAZYNC_REORG_AUTO=1`, the coordinator and the board pass the ack directory above (in the units now), the coordinator
deployed first (a prep that rewinds before the coordinator knows records waits 10 min, then stops for a person).

## 18. Built: P5, the followers (18.11.2026)

Same branch. Host unchanged by P5 (scripts, units, website).

**Offsite copies** (`coordinator/deploy/hazync-offsite-proofs.py`, §9). `--depth N` (the four `hazync-lz-offsite-*`
units pass 6, with the coordinator's `/var/lib/hazync/node_tip`): `copy` uploads a proof only once its hi ≤ tip − 6,
and `check` does not count a younger one missing; `spine` copies the newest KEPT head (`spine/heads/`) at least 6 deep,
never the live one. No fresh tip file (missing, or older than `--tip-max-age`, 1 h): nothing is uploaded and the unit
fails (alerts). For a reorg deeper than that, per coordinator epoch E (`proofs/orphaned/<E>/`): the orphaned proofs go
up under `orphaned/<E>/`, then `reorgs/<E>.json`, written once, names the live keys an orphaned proof holds in the store
(`covered`, each with the orphaned file's sha256 and size). A covered name is never uploaded under its live key again:
the new chain's proof goes to `r<E>/<name>` (the newest covering marker), and `check` compares that key. The spine the
same way (`r<E>/spine_1-<hi>.*` when the store holds that name with another head, explained by a local
`spine/orphaned/<E>/`). **Nothing is deleted** (the mirror stays append-only). A restore takes, per name, the newest
`r<E>/` copy over the live one, reading the markers; the receipts say which chain they are of anyway.

**The zAPP follower** (`hazync-lz-zapp-follow.sh`, §10). Today's follower keeps its own stores with the trie in
memory and the coin log changed in place, and writes no undo records, so it cannot rewind. On a spine below its stores
(or at a height whose block its stores have another of) it checks that the NODE agrees with the spine there (the
block's header hashed), moves its stores to `$ZAPP_ROOT/orphaned/<time>/` (only the newest kept), preps again from
genesis to the spine and republishes; the previous set stays published until then. A node that disagrees with the spine
leaves the stores as they are and fails the run (no rebuild loop). Cost: a prep from genesis, cheap while the spine is
low; the in-memory follower stops scaling near ~140,000 anyway (ZAPP_FOLLOWER §1). New test knobs (`ZAPP_OWNER`,
`ZAPP_RESTART`, `ZAPP_ACTIVE`, `ZAPP_SETTLE_S`) keep the defaults the unit uses.

**The website** (`hazync/hazync-web`, branch `web/reorg-banner`, not deployed): a banner on every page from
`/api/meta.reorg` while the board is `rewinding` and for an hour after `done`; the feed renders `kind: reorg` rows (not
as folds); a block page lists its orphaned proofs, each checkable. `tools/reorg-banner-test.mjs` (+ control) and
`explorer-recent-test.mjs`.

**Harness.** `reorg_e2e.py` runs the offsite `copy --depth 6` and `check` (into an in-memory store) after every
settle and just before each mid-prove reorg: no live name may hold another chain's proof unless a marker covers it, no
marker of a reorg within 6 covers anything, and the depth-W reorg's marker covers names whose new proofs are under
`r<E>/`. The follower runs before the depth-W reorg (serving at the spine), after the fall-back (rebuilt, serving at
the kept head, the served proof the spine) and on the new branch.

**Results, 18.11.2026** (this branch merged with `feat/reorg-p3`, host rebuilt, `46666b6f`): `reorg_e2e.py --w 12`
PASS twice (64 checks each; the depth-12 marker covers 6 live names, their new proofs under `r5/`), `--w 288` PASS (67;
the depth-288 marker covers 282), the control fails 15; `rewind_test.sh` 57/57; `test_offsite_reorg.py` PASS, its
control fails; the coordinator tests pass. One W=12 run before failed in the harness, not the product: the fake node
forgot a block once the test switched its chain, so a fetch worker mid-`getblock` died; a real node keeps stale
blocks, and so does the fake now (`known`).

**Where this differs from the design, and why.**

- **Tip mode is not built** (ZAPP_FOLLOWER Z2-Z5: a read-only reader of the tip run's stores, `zync-serve --at`,
  `PIN`). §7's "repin to the kept head" is a property of tip mode; today's follower instead rebuilds and republishes at
  the kept head. Z2-Z4 are a separate Rust and deployment piece (a reader beside the live service, compared before the
  switch); with them, the rebuild goes away.
- **Orphaned proofs already off-box keep their live names** (§9's markers), rather than being moved: the mirror is the
  backup and stays append-only; the marker says which live keys hold orphaned proofs, and the new ones go to `r<E>/`.
  With the depth delay this happens only for a reorg deeper than 6 below the node's tip.
- **The delay is counted from the node's tip, not prep's**: a reorg at depth 1 below prep's tip while prep is far
  behind the node is deep from the node's point of view, and so is covered by markers, not by the delay.

**Review of 18.11.2026, round 3 (Fable, `05b4a26`), fixed** (after merging `main`, where P1-P3 landed as #754):

- **(HIGH) A reorg inside prep's replay window was never rewound.** After a restart prep replays from its save point A
  while the state files above A from before stay on disk (and served); the fetcher compares only prep's tip, so a fork
  between A and those states read as "above prep's tip": no marker, no record, and the board kept the orphaned blocks.
  Now prep, before applying block h, compares a `state_<h>.json` left by an earlier run with the block file: another
  block is a fork at h-1, handled as any other (it writes the marker, class `auto` within W, and stops; the runner
  rewinds, moves the stale files, writes the record, the board rewinds). `prover/host/src/reorg.rs`
  `stale_state_fork`, host only.
- **(M2) The backstop** checks every verified range in the window (`REORG_AUTO_MAX_DEPTH`) against prep's states, once a
  minute, latches the alert in the database (`meta.reorg_alert`, survives a restart) until a rewind below it is applied,
  is quiet while prep's `out/REWIND` is there, and pages: `hazync-lz-check-reorg.{service,timer}` (every 5 min, fails
  its unit while the alert is latched).
- **(M1) The zAPP follower missed a fall-back** when the spine had fallen back and advanced again past its stores
  between two runs: before continuing from its last save point it now checks the node still has that block, and
  rebuilds if not. The unit gets `OnFailure=` (pages), `HAZYNC_PREP_WAIT_S=120` and `TimeoutStartSec=4h`.
- **Offsite markers wait for the coordinator's `proofs/orphaned/<E>/DONE`** (written after the rewind's commit), so a
  marker is never written from a half-filled directory. `restore-plan` prints, per proof, the key to restore from
  (the newest `r<E>/` copy, else the live name).
- Low: a superseded rewind is marked `done` (A → B → A no longer leaves the first `rewinding`); a start refused by the
  reorg handling (a record that does not read, peers) exits 3, which `hazync-lz-coordinator.service` no longer restarts,
  so the unit fails and alerts; the follower fails when there is no spine but facts are still served; a follower
  rebuild that died before its first save point clears its coin log; the banner is on `sponsors/` and `404.html` too
  (`ooga/` is a pinned copy of `ooga-land` and stays as it is).
- Harness: a reorg inside the replay window (prep restarted, held mid-replay, the node reorgs), the follower missing a
  fall-back, the coordinator killed between a rewind's file moves and its commit (restarted as systemd would) with an
  offsite copy run in that window, and the offsite invariant checked against the NODE's chain.
- Results (host rebuilt from this branch merged with `main`, `46666b6f`): `reorg_e2e.py --w 12` PASS (78 checks), `--w
  288` PASS (81), the control fails 15; with the pre-fix host the replay-window scenario fails (5 checks); the
  reviewer's follower repro now rebuilds and serves the new branch; `rewind_test.sh` 57/57; `test_reorg.py` PASS and its
  seven controls fail; `test_offsite_reorg.py` PASS (+ control); the coordinator tests pass. Building it found one more
  bug, fixed: the L5 shortcut (prep already at the fork) also matched prep's own stop before F+1, and removed the
  marker without moving the stale states; it now requires nothing above the fork.

**Review of 19.11.2026, round 4, fixed.** (M1) The zAPP follower clears its block files, any marker in them, and its
state files above its save point before prepping (a run that died part-way left them, and prep stopped on them for
ever). (M2) The backstop runs in its own `try` in the coordinator's reorg thread and writes a heartbeat
(`meta.reorg_backstop_at`); `hazync-lz-check-reorg` also pages when it is older than 15 min (the coordinator down, or
its thread stuck); the timer is in `scripts/check-deployment.sh` and the RUNBOOK. Low: a state above the save point that
does not read is a stop, not "no stale state"; with static block files (not fed by the fetcher) prep's stale-state stop
writes no marker; an index on `vranges(hi)`; `server.py --reorg-ack [--yes]` clears a latched alert by hand (e.g. prep
restarted from genesis under a fresh `LZ_ROOT`, whose states are not there yet; RUNBOOK); an `out/REWIND` older than
`REORG_REWIND_QUIET_S` (2 h) no longer quiets the backstop; DONE is written for every applied epoch that lacks it, and
an offsite marker deferred over an hour fails the offsite unit; `restore-plan` prints `COVERED-NO-NEW-COPY` for a
covered name whose new copy is not up yet; prep's own marker says the recovery is the rewind, and `fork-point` prints a
marker; the fetcher's depth and orphaned list run to the OLD tip (state files above prep's tip), not prep's tip;
`rewind_test.sh` case L (the replay-window reorg, in CI) with `--control-stale`; `rewind.rs` uses `reorg::hash_of`;
`reorg_e2e.py` prints the offsite output when an offsite check fails.
Results (host from this branch, `46666b6f`): `reorg_e2e.py --w 12` PASS (80 checks), `--w 288` PASS (83), the
control fails 15; `rewind_test.sh` 61/61 and `--control-stale` fails case L (3); `test_reorg.py` PASS and its seven
controls fail; `test_offsite_reorg.py` PASS (+ control); `test_fetch_reorg.py` 43/43 (+ control); the coordinator tests
pass. Two harness races found and fixed on the way (a proof file's mtime a moment ahead of the clock read as "younger"
by `--min-age 0`; the board's tip cache behind its served bundles).

**Review of 19.11.2026, round 5, fixed.** (MED) The round-4 stop on a state file above the save point that does not
read also caught the EMPTY files a power loss leaves (the states between save points are renamed into place without an
fsync): prep stopped with exit 4 where it used to replay over them. A missing or zero-length state is crash damage again
and is replayed over; only a non-empty file that does not parse stops, and the message gives the real recovery (compare
with `fork-point`, move it aside and start prep, or rewind), with no marker. The coordinator stays closed on such a
state (an empty state reads as none: the commit-time check refuses, verify-any has no end state), and the backstop does
not count an empty state as a mismatch (it says how many it skipped). (Low) The `HAZYNC_TEST_SKIP_STALE_CHECK` hook is
refused when prep writes leaf bundles (a board run) and says so on stderr when it fires; `--control-stale` requires the
detection check itself to fail; `--reorg-ack` refuses a `COORD_DB` that does not exist; the backstop records what it
compared (`meta.reorg_backstop_note`), so the check no longer says "on prep's chain" after a quiet run; the RUNBOOK
installs the check timer only after the new coordinator runs. Tests: `rewind_test.sh` case M (an empty state replayed
over, a non-empty unreadable one stopped), `test_reorg.py` (the quiet note, the empty state, the commit still refusing,
`--reorg-ack` with a stray `COORD_DB`).

## 19. Built: D1, the tip run (20.11.2026)

Branch `feat/tip-follow` (on `main` at `cac5a2e`, v0.27.0). Host only on the prep side (`prep.rs`, `main.rs`): no guest
input changed (`scripts/check-guest-inputs.sh`; built embedding the published guest `f7b87bf1`, `host method-id` =
`46666b6f…`, §19 results). ZAPP_FOLLOWER.md §14 has the zAPP side (Z2-Z5), built on the same branch.

**The two runs.** The board pass (`libzync-board-prep`, `LZ_HI=974936`, paced by proving) serves blocks 1..X-1; the
tip run (the paper prep, `libzync-prep`) serves every block from **X = 974,937** and the state X-1 its bundles are cut
against. X is the board pass's `LZ_HI` + 1, so the board pass's units do not change. The same X is written in four
places: the coordinator's `tip-run.conf` (`HAZYNC_LZ_TIP_FROM`), the paper prep's `run.conf`
(`HAZYNC_PREP_LEAF_FROM`), `hazync-lz-check-proofs.service` and `hazync-lz-zapp-follow-tip.service` (`TIP_FROM`).

| piece | built |
|---|---|
| prep, `libzync-prep <lo> follow` | no last block: at the node's tip it waits for the next block file as long as it takes (it says so every 10 min), where a static run times out after `HAZYNC_PREP_WAIT_S`; a block that does not extend the state still times out (the fetcher's verdict, §4). Needs `HAZYNC_PREP_CONSUME=1` (the runner sets it). The runner takes `LZ_HI=follow`; the fetcher's `range 1 follow` existed (P2) |
| prep, `HAZYNC_PREP_LEAF_FROM=<h>` | a leaf bundle for every block from h, beside the `HAZYNC_PREP_LEAF_AT` samples (`prep::wants_leaf`) |
| coordinator: bundles | block b's bundle from the tip run if b ≥ X, else the board pass's (`_bridge_dir_for`); the tip run's sample bundles below X are never served |
| coordinator: states | `state_<h>.json` from the tip run if h ≥ X-1, else the board pass's (`_state_dir_for`, so `lz_state_tip`, the commit check §4 and the backstop read the run that holds h) |
| coordinator: verification | `verify-any` reads one state directory, so each range is verified against a fresh **view**: a directory with `state_<lo-1>.json` and `state_<hi>.json` linked from the run `_state_dir_for` names, as both its state and bundle directory (`_lz_verify_env`). The commit check reads the same files, so the two cannot disagree on which run a height belongs to; a fold across X reads one state from each run. A spine (its height unknown before verification) is verified against the tip run (it holds every state from genesis), then against the board pass if the tip run has no state at its end. `check-proofs.py` builds the same view. **The coordinator's host is unchanged** (`lz_coord.rs`): no host release for the coordinator |
| coordinator: claims | `pick` and `claim` jump the gap no run serves (the board pass's highest bundle .. X): a scan from 1 would otherwise test ~960,000 heights for a file under the claim lock. `TIP_CLAIM_FIRST=1` hands out the tip run's blocks before the backfill's (**off**: a deployment decision, below) |
| coordinator: reorgs | each run's records (`<its out>/reorg/records`) acked in its own directory (`HAZYNC_LZ_TIP_REORG_ACK_DIR`; prep's `HAZYNC_REORG_ACK_DIR=/var/lib/hazync-lz/reorg-acks-tip`), applied as `src` `prep-tip`, its intent (`out/REWIND`) and its moved states looked up in its own directory. The two runs number their epochs independently: one ack directory for both is refused at start-up (exit 3), as are a missing directory and one run's directory given for the other |
| coordinator: a fork below X - 1 | a tip-run record whose fork is below X - 1 while the board pass holds the block above it is **refused, not acked** (loud): the board pass's states and bundles above the fork would still serve and re-verify the orphaned blocks. Practically impossible (the board pass reaches X - 1 months after it is deep), handled by a person (stop both, rewind the board pass with §11 step 3 under its own `LZ_ROOT`, then the tip run) |
| coordinator: the handover check | the backstop (every minute) compares the two runs' `state_<h>.json` at the board pass's newest height ≤ X - 1 (its `PREP_TIP.json`); a difference latches `reorg_alert` ("disagree on state_<h>.json"), which `hazync-lz-check-reorg` pages on. Both runs prep the same chain from genesis with the same host, so their states are byte-identical where both exist (MEASURED on hazync-proof 20.11.2026 at 1, 5,000, 8,000 and 8,700) — the joins across X depend on it (`link_digest`) |
| `/api/meta.tip_run` | `{from, servable_high, board_servable_high, prep_tip: {height, age_s}, claim_first}`; null with one run |
| the stall check | `hazync-lz-check-reorg` with `CHECK_TIP_RUN_PREP_TIP`: the tip run's `PREP_TIP.json` unchanged for an hour (`CHECK_TIP_RUN_STALE_S`) while the node's tip is above it pages (prep in `follow` has no timeout of its own) |
| leaf pruner | the tip run's bundles sit above the frontier until the backfill reaches X, so the frontier rule never frees them: `TIP_LEAVES` deletes a tip bundle once its block is proven (`/api/blockstatus`) and at least `TIP_DEPTH` (6) below the node's tip (`/var/lib/hazync/node_tip`, fresh within an hour, else nothing goes and the unit fails) |
| units | `libzync-prep.service`: `HAZYNC_REORG_ACK=0` removed, `HAZYNC_REORG_ACK_DIR=/var/lib/hazync-lz/reorg-acks-tip`; `libzync-prep` and `libzync-fetch` `run.conf`: `LZ_HI=follow`, `HAZYNC_PREP_LEAF_FROM=974937`; the coordinator's `tip-run.conf` drop-in (new); `hazync-lz-check-proofs` and `hazync-lz-prune-leaves` get the tip run's variables |

**Decisions taken here, for review.**

1. **X = 974,937**, the board pass's static `LZ_HI` + 1, so the board pass needs no change and no reorg can reach it
   (it gets there months from now, ~1,800 blocks below today's tip already). The paper prep must write bundles from X
   on, so `HAZYNC_PREP_LEAF_FROM` must be set before it passes X (it is at ~446,000).
2. **The coordinator's host is not changed.** Two state directories could be taught to `verify-any` (a host change and a
   release for the coordinator); the per-call view gives the same result from the coordinator side alone.
3. **`TIP_CLAIM_FIRST` defaults to off.** The spec does not say who proves tip blocks. With it off, normal workers take
   the backfill first (the board pass always has bundles open, so tip blocks go to workers only when the backfill is
   exhausted, or to the sponsor bot / `hazync run <n>`); with it on every new tip block is claimed first, by whoever
   asks, and a mainnet tip block is far heavier than the backfill's (~200 s on 4 RTX PRO 6000s, MEASURED in Phase 5).
   **Open: the user's call.**
4. **A tip-run fork below X - 1 is refused**, not handled (above).
5. **No leaf window on the tip run.** Pacing the tip run by proving (the board pass's 5,000) would also hold back its
   states, which the coordinator verifies against; at the tip it would stop following. Its bundles are bounded by
   proving at 144 a day and the pruner above; the disk floor (`HAZYNC_PREP_MIN_FREE_GB=200`) stops it with exit 3.

**Automatic rewinds stay OFF** (`HAZYNC_REORG_AUTO` unset in every unit). At the tip a reorg stops the paper prep and its
fetcher with exit 4 (they fail and alert); the recovery is §11 step 3 under the paper prep's root, and the coordinator
acks it (it follows the tip run's records):

```bash
systemctl stop libzync-fetch libzync-prep
cd /srv/bulk/libzync-prep && HAZYNC_COINLOG_DIR=$PWD/cl HAZYNC_TRIE_DIR=$PWD/trie HAZYNC_PREP_LEAF_BUNDLES=$PWD/leaves \
  HAZYNC_PREP_UNDO_KEEP=2016 HAZYNC_REORG_ACK_DIR=/var/lib/hazync-lz/reorg-acks-tip \
  bin/host_lz libzync-prep-rewind $PWD/out $PWD/blocks          # --allow-deep for a `manual` marker (deeper than W)
systemctl start libzync-prep libzync-fetch
```

Turning automatic rewinds on is a deployment decision after the reviews (§17): `HAZYNC_REORG_AUTO=1` on
`libzync-prep` and `libzync-fetch` (the paper prep, now the tip run), not the board pass.

**The paper prep's one restart.** It runs the v0.26.1 host (MEASURED 20.11.2026: `host_lz libzync-prep 1 974936 …
genesis`, started 16.11, ~446,000 at ~0.32 blocks/s, 41-43 GB RSS); v0.27.0's `host_lz` is installed in its `bin/` but
it was never restarted, so it writes no undo records, no `PREP_TIP.json` and no Z1 roots. Following the tip needs a host
from this branch (`follow`, `LEAF_FROM`) and the new `run.conf`, so it restarts ONCE, from its newest save point
(`HAZYNC_PREP_CONTINUE=1`, the runner does it). Cost (ESTIMATE from the measured rate): the blocks since its last save
point replayed, at most 250 (`HAZYNC_PREP_SAVE_EVERY`), so up to ~13 min at 0.32 blocks/s, plus the start (the BIP30
tree from the `.smt`, the 12 GiB trie record cache cold: the first thousands of blocks run slower until it refills; the
v0.26 reset measured the same pattern). Undo records then fill J = 2,016 blocks in ~1.75 h at that rate (W = 288 in
~15 min), long before the run nears the tip (~20 days). From the restart on it also logs Z1 roots (`trie/roots.log`,
`GEN`) and `PREP_TIP.json`. Its stores are on `/srv/bulk` (vdb: `cl` 21 GB, `trie` 94 GB at 448,000, MEASURED), not on
vda as §2 and ZAPP_FOLLOWER §8 assumed; only `out/` is on vda (`/var/lib/libzync-fast/out`). The pre-v0.26 run's stores
on vda (`/var/lib/libzync-fast/{cl,trie}`, 54 GB + 279 GB, reached by the `*.pre-v026` symlinks) are not the paper
prep's current stores.

**Tests.**

- `coordinator/test_reorg_tip.py` (CI, seconds): configuration refused at a real start-up (exit 3); bundles and the gap;
  claims jump the gap (no height in it tested), `TIP_CLAIM_FIRST`; leaves of 25 and of X (start state X-1 the tip
  run's), a fold across X (one state from each run), spines; `check-proofs.py` with and without the tip run's
  variables; the handover check latching; a tip-run record acked in its own directory and the board pass's record of the
  same number in its own; a fork below X - 1 refused; `/api/meta.tip_run`. Five controls, each failing:
  `--control-view` (4 checks), `--control-acks` (4), `--control-gap` (1), `--control-handover` (1), `--control-belowx`
  (1).
- `coordinator/test_prune_leaves_tip.py` (+ `--control`, the pruner without its tip section: 2 fail).
- `prover/host` `prep::tests::leaf_bundles_from_the_handover_height`.
- `experimental/libzync-bench/prep/reorg_e2e_tip.py` (P6 for the tip run). **Results, 20.11.2026** (hazync-coord, a
  fasthost build of this branch embedding the published guest, `host method-id` `46666b6f…`; zync-serve built with
  `--features tip --profile zync-serve`): **PASS, 29 checks** (A-E: the board pass stops at 149, the tip run follows with
  no restart, the frontier crosses X, zync-serve serves the tip run's trie at the spine, a depth-2 and a depth-12 reorg
  of the tip run rewound with no person and acked in `acks-tip/` only, the follower repinning to the kept head and
  serving two heights after it). `--control` (the coordinator without the tip run's variables): 3 checks fail (the
  frontier stops at 149), as they must. The first run found the follower handing its lock to the zync-serve it
  started (fixed, review table below).
- Unchanged and passing: `test_reorg.py` (+ its seven controls), `test_reorg_units.py`, `test_libzync_coord_files.py`,
  `test_check_end.py`, `test_spine_seed.py`, `test_claim_sybil.py`, `test_zync_relay.py`.

**Risks.**

- The four copies of X must agree. A coordinator X above the paper prep's `LEAF_FROM` leaves blocks below it unserved
  (the board pass stops at 974,936); below it, the coordinator looks for tip-run bundles that were never cut.
- `LZ_HI=follow` with an OLD runner script reads as "already at the end" (`[ 1 -le follow ]` is an error, its `||`
  branch exits 0): the unit stops quietly. Install the runner and the host BEFORE the `run.conf`; an old host refuses
  `follow` with a panic (exit 101), which systemd restarts every 30 s for ever (no alert). The procedure orders it.
- At the tip, prep in `follow` never exits for a stalled fetcher: a hung fetcher shows only as the tip run's
  `PREP_TIP.json` not moving. `hazync-lz-check-reorg` (`CHECK_TIP_RUN_PREP_TIP`) pages when the node has been ahead of
  it at the same height for an hour (tested in `test_reorg_tip.py`); `/api/meta.tip_run.prep_tip.age_s` shows it.
- `verify-any`'s view adds a directory and two symlinks per verification (under `COORD_STATE`), removed after it; a crash
  in between leaves `*.view` directories there, swept at the next start.
- A spine is verified twice with a tip run (its height, then its end against the one state the view holds).

**Review of 20.11.2026 (independent, `feat/tip-follow` at `56cee3b`), fixed.** No soundness break was found.

| # | finding | assessed | fixed |
|---|---|---|---|
| 1 HIGH | the runner clears `cl/` and `trie/` and starts from genesis when it finds no complete save point; the paper prep's first start under it would do that to weeks of prep if it could not see its save points | agreed | the runner refuses (exit 3: the unit fails and alerts) to clear stores when `out/` holds any state file or the run is `follow`, unless `LZ_PREP_CLEAR_STORES=1`; `LZ_RUN_DRYRUN=1` prints the save point a start would take and changes nothing; the RUNBOOK runs it before the restart. MEASURED read-only on hazync-proof (the new runner, dry run): `would continue from state_449750.json (449751..follow); trie meta at 449750`. `coordinator/test_prep_runner.py` (+ control: 3 fail). A genesis run that dies before its first save point now stops for a person instead of clearing itself |
| 2 MEDIUM | with automatic rewinds off, a reorg stops the tip run with `blocks/REORG`, but the board kept serving, verifying and crediting its orphaned bundles and states above the fork | agreed | **the fence**: each run's block directory (`HAZYNC_LZ_BLOCKS_DIR`, `HAZYNC_LZ_TIP_BLOCKS_DIR`); a `REORG` marker fences that run above its fork, `REORG_TOO_DEEP` without a fork (or a marker that does not read) all of it: no bundle served or claimed, `lz_state_tip` none (the commit check refuses), a submission above it 409 `reorged`, not counted. Lifted when the marker goes (the rewind). `/api/meta.tip_run.fences`. Not done: serving tip bundles only k deep (it delays every tip block for a rare event the fence already covers) |
| 3 MEDIUM | tip bundles unbounded while nobody proves tip blocks | agreed | `HAZYNC_PREP_LEAF_WINDOW=4320` on the paper prep (30 days): prep PAUSES before its next bundle, nothing deleted; the stall check says when the window is why. Sizes MEASURED: 9.6 MB at 481,824 (the largest sample bundle; none exists near the tip yet), `/srv/bulk` 3.3 TB free; runway without a window ~1,800 days at 144 × ~12 MB/day (ESTIMATE), with it ~40-65 GB. `TIP_CLAIM_FIRST` unchanged (off) |
| 4 MEDIUM | the RUNBOOK never installed the fetcher; the stall check paged before `PREP_TIP.json` existed | agreed (the box's fetcher is already the repo's, MEASURED same md5) | step 2 installs `fetch_block_rpc.py` and checks it has `follow`; step 4 checks both units active and the block files moving, after step 5's checks; the stall check says "not checked" with no `PREP_TIP.json` |
| 5 MEDIUM-LOW | deleting `tip-run.conf` after tip blocks are verified latches the alert and fails `check-proofs`; the paper prep would wait for acks nobody writes | agreed | the RUNBOOK's full rollback set (`--reorg-to 974936` first, then the drop-in, `HAZYNC_REORG_ACK=0` back, the checks' variables); the drop-in says so. `test_reorg_tip.py` shows both orders |
| 6 LOW-MEDIUM | the handover alert latched for ever and blocked the other latch | agreed | its own key, `handover_alert`, cleared when the runs agree again; `hazync-lz-check-reorg` pages on it (+ control) |
| 7 LOW-MEDIUM | a missing, empty or torn boundary state counted against the block | agreed | 503 before verifying, not counted (+ control) |
| 8 LOW | the spine was verified against the tip run's directories (and its leaf bundles) even below X - 1 | agreed | its height first, then `--check-end` against a view of the one state from the run that holds it (+ control) |
| 9 LOW | the below-X refusal tested "the board pass holds F+1", not the orphaned F+1 | agreed | compared with the record's orphaned hash (+ control) |
| 10 LOW | a reader whose root prep re-logged elsewhere answered 503 for ever; history walks up to the cap | agreed / kept | `Reader` follows the same root hash at its new reference (`z2_a_relogged_root_is_followed`); the follower probes every served height and restarts zync-serve on a 503 (fails if it persists). The 100,000-record cap is the user's decision (ZAPP_FOLLOWER §12.4) and is a unit setting (`ZYNC_WALK_CAP`): not changed, its cost stated |
| 11 LOW | X in four places unchecked; the backstop's window counted from the tip; views not swept; the gap jump on an unreadable board directory; the stall check after a quiet hour | agreed | the follower and `check-proofs` refuse an X that is not the coordinator's `tip_run.from`; the backstop and `REORG_AUTO_MAX_DEPTH` count from each run's own top (+ control); views swept at start-up; the last board high kept; the stall check measures how long the node has been ahead (a state file) |

Also found: `test_reorg.py --control-m1` had stopped applying since this branch's first commit (its patch named the old
`_prep_waiting_on` call); fixed. Suspicion (a), the pinned core wasm: built in the published image
(`HAZYNC_WASM_REQUIRE_CLEAN=1 HAZYNC_WASM_REQUIRE_PUBLISHED=1`), both before and after keeping the line numbers, the
core `.wasm` is `0c8cf026…`, the pin (MEASURED): the shifted lines had not moved it, and the reader now sits at the end
of `trie_store.rs` so no later change above it can. (c) vda: 1.4 TB free (MEASURED); the paper prep's `out/` adds ~3.7
KB a block plus its 2,016 undo records (~2 GB), and the board pass's retention is §8's estimate; the disk floor watches
each run's coin-log filesystem (vdb for the paper prep, vda for the board pass).
