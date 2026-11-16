# Following the tip with zAPP facts (design)

> **Design, 16.11.2026; Z1 built the same day** (branch `feat/reorg-p1-p2`, with REORG P1; §13). The rest is not built. I wrote it from the repo at `797b777e` (main), the reorg design on
> `design/reorg-handling` (`docs/libzync/REORG.md`, cited as REORG §n) and a read-only survey of `hazync-proof` the
> same day. **MEASURED** numbers come from that survey or the files named next to them; **ESTIMATE** numbers are
> labelled. The user's decisions on the open questions are in §12.

The user decided (REORG §14, questions 1 and 6): the **paper prep** (`libzync-prep`, `LZ_ROOT=/srv/bulk/libzync-prep`)
is the run that follows the tip, and zAPP facts at the tip read **that run's stores**. This page covers how the fact
service (`zync-serve`) does that without its own copy of the chain state, how it stays on the height the board has
proven while prep runs ahead, and what happens on a reorg.

## Decisions at a glance

| | proposal |
|---|---|
| what is served at the tip | **counting-trie facts only** (`/tip`, `/trie/balance`, `/trie/history`, `/trie/coin`): the only paths the coordinator relays (`server.py` `_ZYNC_PATH`) and the only ones the website uses (`assets/zapp.js`). `/coin` and `/find` (coin-log facts from `history.bin`) are not public and stay with local `zync-path` users (§3) |
| how | **pinned roots**: the trie arena is already copy-on-write. Prep records the arena root of every block (`trie/roots.log`) and holds back the slots a pinned root still needs (`trie/PIN`, `trie/retain/`). `zync-serve` maps `nodes.bin` **read-only** and walks from the root of the proven block (§4) |
| height served | the **spine's** `hi` (a genesis-anchored proof), never prep's tip. Bound by `verify-any --check-end` against the source run's `state_<H>.json`, then by the trie root in that state (§5) |
| source run | the run whose state files the coordinator checks `H` against: the board pass while it holds `H`, the paper prep above the D1 handover height **X** (§6) |
| retention | at the tip, **W = 288 blocks** of replaced slots (ESTIMATE ~7 GB on vda), so every head a W-deep reorg can fall back to stays readable (§7) |
| image id | **unchanged**. Host-only (`trie_store.rs`, `trie.rs`, `zync_serve.rs`), scripts, units, website. No existing on-disk format changes; files are only added (§4) |

## 1. What zync-serve needs today (from the code)

`zync-serve <history.bin> <addr:port> [state_<tip>.trie]` (`swiftsync-verify/src/zync_serve.rs`):

| input | how it is used | at mainnet scale |
|---|---|---|
| `history.bin` (`libzync/src/history.rs`: per height the block hash, live value, flips, appends, undo hash) | read whole and decoded into a `Vec<Rec>` (`:79`); `tip = len − 1`; `replay(tip, tip)` at start builds `coins`, every leaf ever (`:81`) | ESTIMATE ~58 GB at 635,000 (1.25 billion leaves × 32 B + flips × 16 B) and ~130 GB at the tip. Cannot be held in memory |
| `/coin/<at>/<pos>` | `replay` **from genesis on every request** (`:22`) | O(chain) per request |
| `/find/<leaf>` | linear scan of `coins` (`:49`) | O(leaves) |
| `state_<tip>.trie` (snapshot `HZTRIE01`) | `TrieStore::from_bytes` into an in-memory arena (`:83`), root recomputed and checked; must be at the history's tip (`:85`) | 80 B per node in memory (MEASURED with `size_of::<Node>()`), 4.08 billion nodes at 635,000 (MEASURED) = 327 GB |
| `/trie/*` | `make_trie_fact` (`swiftsync-verify/src/lib.rs:928`): `prove_coins` / `prove_addr` / `prove_history`, which only **read** nodes | a walk of ~37 nodes per key (§8) |

The unit runs it with `MemoryMax=1G` (`hazync-lz-zync-serve.service`). The follower (`hazync-lz-zapp-follow.sh`) preps
its own stores to the spine's height with the trie **in memory** (a disk trie writes only an `HZTRIE-DISK` marker,
which `zync-serve` cannot load) and publishes `history.bin`, `state.trie` and `proof.rcpt`. The coordinator serves
`proof.rcpt` as `/api/zync/proof` (`ZYNC_PROOF`) and relays only `/tip` and `/trie/*`.

**Where today's path stops (ESTIMATE from MEASURED node counts).** `PREP_PROGRESS` on the paper prep gives trie nodes by
height: 2.6 M at 125,000, 12.2 M at 150,000, 47.7 M at 200,000, 146 M at 250,000. At 80 B in memory plus ~40 B of
snapshot bytes while loading, the trie alone passes `zync-serve`'s 1 GB cap between 125,000 and 150,000. With the cap
raised, the box (62 GB, 21 GB available beside the paper prep at 39–48 GB RSS) runs out between 200,000 and 250,000:
at 250,000 the trie is ~12 GB in `zync-serve`, ~12 GB again in the follower's own prep, plus the history replay. Today that is far off: the spine is at
**1,999** and no proof has been stored since **14.11** (MEASURED: `spine.json`, proof file dates).

**On a spine rewind** the follower `die`s ("the spine is below this follower's stores", `:44`) and `zync-serve` keeps
serving the old, now orphaned, height until someone gives it a fresh `ZAPP_ROOT`.

## 2. What the paper prep has on disk (MEASURED, 16.11.2026)

| store | where | size | a reader at height H < prep's tip P |
|---|---|---|---|
| trie arena `nodes.bin` + `meta` | `/var/lib/libzync-fast/trie` (vda) | 314 GB file; meta at 635,000: `len` 4.45 billion slots, root ref 4,423,051,374, **367.5 M free slots** (meta is 2.94 GB) | copy-on-write: R_H stays intact as long as no slot reachable from it is reused. Today only the flushed root is recorded, and freed slots are reused after the next flush |
| coin log `L*.bin`, `index.bin` | `/var/lib/libzync-fast/cl` (vda) | 55 GB (L0 40 GB, index 10.7 GB) | changed **in place** (flips overwrite leaves). The log at H means undoing (H, P] with REORG's undo records |
| `history.bin` | — | **does not exist** on either prep run (`HAZYNC_PREP_HISTORY` is not set) | history must be contiguous from genesis, so it cannot be switched on now |
| `state_<h>.json` | `/var/lib/libzync-fast/out` | one per height, 3.7 KB (REORG §2) | carries `trie_root` and `tip_hash` for every h |

The trie is the only store a reader at H can use without undoing anything, and the trie is all the public paths need.

## 3. Trie only at the tip

The website asks only for trie facts (`zapp.js` `ask()`: `/zync/trie/{balance,history}`), and the relay passes nothing
else. A trie **coin** fact (`/trie/coin/<spk>/<txid>/<vout>`) already answers "is this output unspent, who spent it, or
did it never exist", which is the question a coin-log `CoinFact` answers. So I propose that tip mode drops `/coin` and
`/find` and does not need `history.bin` at all.

Keeping coin-log facts at the tip would need (a) a history for the tip run from genesis, which the paper prep never
wrote (a genesis replay, ESTIMATE 8+ days as in `RESET_v026.md`), (b) a disk-backed history reader instead of the
whole-file decode, and (c) `hist_append` changed too: on its first append it reads the whole file into memory
(`lz.rs:1758`), which at ~58 GB would not fit. I leave that out (open question 3).

## 4. Pinned roots

The arena already has the property this needs (`trie_store.rs` header): an update writes new nodes along its paths and
frees the replaced ones; a freed slot goes to `pending` and becomes reusable only at the next flush (`DiskArena::durable`).
So **every root since the last flush is intact today**, until the first `put` after the next flush. Three additions
make that window explicit and long enough:

1. **`trie/roots.log`** (prep, `trie.rs` `trie_commit`): after each accepted apply, append
   `height u32 ‖ block hash 32 ‖ root ref u64 ‖ root hash 32 ‖ gen u32` (80 B, no fsync). `gen` comes from
   `trie/GEN`, which prep increments and syncs **every time it opens the arena** and on every rewind; at open it also cuts
   entries above the flushed height (those slots will be reused by the replay).
2. **Retention** (`DiskArena`): `free(r)` records the height that freed the slot. At `durable()`, slots freed by blocks
   `k ≤ release` go to the free list as today; the rest are written to `trie/retain/r_<k>.bin` (8 B per slot, synced
   before the meta rename) and are not handed out, also not after a restart. `release = max(min(PIN, P), P − RETAIN_MAX)`.
   **With no `trie/PIN` file, `release = P`: today's behaviour exactly**, so the change is inert until a reader pins.
3. **Read-only reader** (`DiskArena::open_read`, `TrieStore::open_at(dir, h, hash)`): opens `nodes.bin` read-only
   (`Mmap`, shared, so it sees prep's writes through the page cache; it never takes `LOCK`), finds `(h, hash)` in
   `roots.log` with the current `gen`, and checks every reference against the file's current length (the mapping runs
   past the end; a reference beyond it must be an error, not a SIGBUS). `put`/`free` panic.

**The invariant.** A slot reachable from R_H is freed only by a block above H. Prep reuses a slot only once its
freeing height is ≤ `PIN` (or older than `RETAIN_MAX`). So while `PIN ≤ H`, nothing reachable from R_H is overwritten,
and the reader never sees a torn record.

**A crash of prep.** The arena falls back to its last flush f; slots allocated above f are reused by the replay. The
`gen` bump at open happens before any write, so a reader checks `gen` before and after each walk (one `stat` of a 4-byte
file) and answers 503 if it changed; roots ≤ f stay valid (their retained slots are in `retain/`, not in the free list).
A walk that reads garbage anyway can only produce a fact the client refuses (§5), and `zync-serve` catches the panic
from a bad record tag instead of dying.

**Pinning.** The follower writes `trie/PIN` (`height`, time) atomically: the **lowest** height `zync-serve` may still
need (§7). If the follower dies, `RETAIN_MAX` bounds the disk; prep logs and alerts when the cap releases a pinned
height, and records `released_through` in `trie/PIN.released`, which the reader checks before serving.

## 5. Which height, and why it is the right one

Facts are checked in the browser against a genesis-anchored proof of 1..H, and the page refuses a fact whose `at` is
not the proof's height (`zapp.js`). So H is the **spine's `hi`**, never prep's tip P. Prep runs ahead by the proving
latency, and that gap is what retention covers. Each follower run:

1. copies the spine, then runs `HAZYNC_LZ_STATE_DIR=<source out> host verify-any <spine> 1 --check-end`. The end check
   (`lz_coord.rs:136`) requires the receipt's end state to be the source run's `state_<H>.json` in every field
   (`link_digest`) and its tip hash: so that file's `trie_root` is the root the proof commits;
2. finds `(H, tip_hash)` in the source run's `roots.log`. Not there yet (prep has not applied H, or `gen` moved): keep
   serving the old height and try again next run;
3. starts `zync-serve` on `(H, hash)`; `zync-serve` checks its computed root against the `trie_root` passed in and
   refuses to start otherwise; the follower restores the previous set if it does not stay up, as today;
4. publishes `proof.rcpt` (the copied spine), then lowers `PIN` to the new floor.

The chain of binding is proof → `state_<H>.json` (end check) → `trie_root` → the served root. None of this is trusted by
the page; it only stops the service from serving facts the page would refuse.

**At-height serving.** Today a page that loaded the proof at H and asks after the follower moved to H' is refused
("another height"). The follower runs every 10 min, so this is rare now, but at the tip it would be every block. I
propose `zync-serve` keeps the **two newest** pinned heights and takes `?at=<H>` (default the newest); the relay regex
allows it; `zapp.js` sends `r.height`. `PIN` is then the older of the two (or lower, §7).

## 6. Source run: board pass below X, paper prep above

The user's decision covers the tip. Below the tip the paper prep cannot help: it is at 635,000 while the spine is at
1,999, and retaining 633,000 blocks of replaced slots is out of the question. The run that holds the spine's height
during catch-up is the **board pass** (`libzync-board-prep`, stores on vda at `/var/lib/libzync-board`): its leaf
window (`HAZYNC_PREP_LEAF_WINDOW=5000`) keeps it at most 5,000 blocks ahead of the frontier (MEASURED: 6,011 states
against a frontier of 1,999).

So the rule is: **the source for H is the run whose state files the coordinator checks H against.** Below the D1
handover height X that is the board pass, above it the paper prep (REORG §3). The mechanism is the same host code on
both runs; the follower only picks the directory. When the spine crosses X the follower switches, and the board pass's
`PIN` is removed (its stores can then go, see §8).

The alternative for catch-up is to keep today's path until it breaks (~140,000, §1) and then **freeze** Coin check at
that height (the proof 1..H stays valid and is still served) until the tip run takes over. It costs nothing but leaves
the page on an old height for as long as the board takes to cross X (open question 1).

## 7. Reorgs (REORG P3/P4)

| case | what happens |
|---|---|
| fork F ≥ H (the usual case: a depth-1 to depth-6 reorg while H lags P by the proving latency) | nothing for the reader. Prep's rewind (REORG §6 B) undoes blocks above F **copy-on-write**: it frees only slots created above F, which R_H never referred to. Prep bumps `gen`; the reader re-resolves `(H, hash)`, which is still in `roots.log` |
| F < H (facts were being served about orphaned blocks) | REORG P4 sets the spine back to the newest kept head H′ ≤ F (`spine/heads/`). The follower sees the spine's `hi` and `out_bhash` change, finds `(H′, hash)` in `roots.log`, and repins. R_H′ is readable only if H′ ≥ `PIN`: so at the tip `PIN` is held at **the oldest kept spine head within W of P**, not just the older served height. Cost: W × replaced slots per block (§8) |
| H′ below what is retained (only past W) | 503 from `zync-serve`; the follower serves again once the board proves a new head on the new branch. Prep's stop-and-alert path (REORG §11) applies anyway |
| facts already served about orphaned blocks | nothing to retract. Each one is a true fact about a valid chain, checked against the proof the page loaded, whose tip hash it shows. The banner from `/api/meta.reorg` (REORG §7) covers the website |
| `roots.log` entries above F | prep cuts them in REORG §5 step 7 (copied into `reorg/` with the epoch, for the record). Entries are keyed by `(height, block hash)`, and the follower asks by the spine's hash, so an orphaned root cannot be served for a new-chain height even before the cut |

Ordering: prep's step 7 must not reuse slots of an orphaned root the reader may still be walking. The `gen` bump at the
start of the rewind makes the reader stop answering for heights above F before any slot is reused; the reader's
before-and-after `gen` check covers a walk in flight.

**What changes in REORG.** §10's zAPP follower item (run `libzync-prep-rewind` on its own stores, cut `history.bin`)
goes away: in tip mode the follower has no stores. P5's follower part becomes "repin to the kept head", and P3 gains
two lines (bump `gen`, cut `roots.log`). `roots.log` also gives P3 a free cross-check: after `undo_recorded` to F, the
trie root must equal the root hash logged at `(F, hash)`.

## 8. Costs

**Replaced slots per block (MEASURED, derived).** The paper trie's meta at the 635,000 flush lists 367.5 M free slots.
In steady state the interval's allocations drain the previous free list (live nodes grew 16.1 M per 1,000 blocks
between 625,000 and 635,000 while the file kept growing), so that is one interval's garbage: **~367 K slots per block,
~23.5 MB**. It matches the model: 4,916 new coins per block (MEASURED, coin-log leaves 625,000→635,000) ≈ 9.8 K trie
updates × ~37 nodes per path × 64 B.

| item | where | size |
|---|---|---|
| `roots.log` | trie dir, vda | 80 B per block: ~78 MB for the whole chain |
| retention at the tip, W = 288 | inside `nodes.bin`, vda | ESTIMATE ~25 MB per block × 288 ≈ **7 GB**, plus 288 × 2.9 MB ≈ 0.85 GB of `retain/` lists |
| retention on the board pass during catch-up (window 5,000 + spine lag) | vda | ESTIMATE by height, at ~2 updates per new coin: ~1 MB/block at 200,000 → **5 GB**; ~4.7 MB at 300,000 → **23 GB**; ~23.5 MB at 600,000 → **117 GB** + 15 GB of lists. A smaller leaf window shrinks it in proportion |
| `zync-serve` memory | — | page cache only, charged to its cgroup and reclaimable: `MemoryMax=1G` stays (ESTIMATE). It has nothing to load at start |
| read cost per fact | vda | ESTIMATE ~37 random 4 KiB reads for a balance or coin fact; ~8 ms at the ~5 K serial random reads/s noted in `trie_store.rs`. A history fact reads the script's whole coin trie: needs a cap (§10 Z2) |
| contention with prep | vda | prep is read-bound on the trie (MEASURED `PREP_TIMING` at 635,000: trie 3,365 s of 5,570 s per 1,000 blocks, 6.47 M major faults). Website traffic is a handful of walks; `zync-serve` is single-threaded, so it is at most one walk at a time |
| `/srv/bulk` | — | nothing write-hot goes there. `roots.log`, `retain/` and `PIN` live in the trie directory on vda (`/var/lib/libzync-fast/trie`, `/var/lib/libzync-board/trie`). The follower's `tmp/` (one 229 KB spine copy) can stay on `/srv/bulk` |

**Disk on vda at the tip (ESTIMATE, linear from the last 10,000 blocks).** The paper trie grows by ~16 M nodes per
1,000 blocks: ~9.6 billion nodes at 976,000 ≈ 613 GB live, ~690 GB with the 1/8 growth step; the coin log ≈ 130 GB.
About **820 GB** for the paper prep, against 1.4 TB free today (MEASURED). The board pass grows towards the same size
as it catches up: both at the tip would leave under ~0.2 TB of vda's 2 TB free, before retention. So the board pass's
stores should go once the spine crosses X. That belongs to D1, but this design relies on it.

## 9. Rejected options

| option | why not |
|---|---|
| today's path (own prep, trie in memory) | stops between ~140,000 and ~250,000 (§1) |
| the follower's own prep with a **disk** trie, trailing the spine (prep exactly to H, read at its flushed root) | works with no change to the tip run, but is a third copy of the stores: ~345 GB at 635,000 (MEASURED sizes), ~820 GB at the tip (ESTIMATE). It does not fit on vda beside the paper prep, and on `/srv/bulk` its random reads compete with the ~40 MB/s write cap. It contradicts the decision; I keep it as the fallback if retention misbehaves |
| copy the trie at each proven height | 314 GB per copy (MEASURED) |
| filesystem snapshots or reflinks | both disks are ext4 (MEASURED `findmnt`), plain partitions, no LVM |
| serve at prep's flushed root with no prep change | the flushed root is only at multiples of 1,000; a genesis-anchored proof would have to end exactly there, and the root is gone at the next flush |
| coin-log facts at H by undoing (H, P] from REORG's undo records | possible for `at = H` (sibling hashes recomputed along the flipped paths), but a spent coin's **original** leaf is not on disk without a history, and the trie coin fact answers the same question (§3) |

## 10. Phases

| phase | content | size (ESTIMATE) | ship |
|---|---|---|---|
| **Z1 roots and retention (prep)** | `roots.log`, `GEN`, per-height `pending`, `retain/`, `PIN`/`RETAIN_MAX`, cut on open. Inert without a `PIN` | ~200 lines Rust + ~200 test | **with REORG P1**, in the same host release: one restart of each prep run (continue from its save point). Roots exist only from that restart on, so it must come before the reader needs them |
| **Z2 reader (`zync-serve` tip mode)** | `DiskArena::open_read`, `TrieStore::open_at`; `zync-serve --trie-dir <d> --at <h>:<hash>:<trie_root>` with no history; `gen` checks, caught panics, a walk cap for history facts (refuse past N nodes with a clear error, instead of the relay's 8 MB 502) | ~200 lines Rust + ~150 test | after Z1 |
| **Z3 follower switch** | the follower rewritten: no prep, `--check-end` against the source run, `roots.log` lookup, `PIN`; the unit gets `ReadOnlyPaths` for the two trie dirs. First run **beside** today's service on another port and compare answers at the same H, then switch the relay | ~80 lines shell (replaces 97) | once the spine passes the board pass's first rooted height |
| **Z4 at-height serving** | two pinned heights, `?at=`, the relay regex, `zapp.js` sends `r.height` (web repo) | ~60 lines + ~5 JS | before the follower runs more often than every 10 min |
| **Z5 tip source and reorgs** | source = paper prep above X; `PIN` at the oldest kept head within W; repin on a spine fallback; `gen` bump and `roots.log` cut in prep's rewind | ~60 lines shell + ~30 Rust | with REORG P3/P4/P5 |
| **Z6 (optional) coin-log facts at the tip** | history for the tip run from genesis, a disk-backed reader, `hist_append` without the whole-file read | not sized | only if asked (open question 3) |

**Tests.** Z1/Z2: random applies on a disk arena with a pin; after several flushes and a reopen, every root ≥ `PIN`
gives facts byte-identical to an in-memory store at that height (`to_bytes` of each), and a reused slot is never one
reachable from a pinned root. Crash: kill prep between flushes; roots above the flush answer 503 after the restart,
roots below still match. On real data: prep a regtest or mainnet 1..200 run on disk with a pin at 150 and run to 200;
every fact at 150 must equal today's in-memory `zync-serve` on `state_150.trie`. All CPU, seconds to minutes, local or on
the box; no Actions run without asking.

## 11. What breaks for the website and zAPPs during the transition

1. **Nothing until the spine reaches ~140,000** (ESTIMATE), on today's path. Proving has been idle since 14.11, at 1,999.
2. **The switch needs roots for H.** The board pass is ~4,000 blocks ahead of the spine; after the Z1 restart its
   roots start at that height, so today's follower keeps serving until the spine passes it. Then Z3 switches, after
   the side-by-side comparison.
3. **`/coin` and `/find` disappear** from `zync-serve` in tip mode. They are not relayed, so the website and the public
   API do not change; anyone using `zync-path` with a local `history.bin` is unaffected.
4. **More "another height" refusals** if the follower runs more often before Z4 ships. Keep the 10-min timer until Z4.
5. **Huge histories** (scripts with very many coins) get a clear refusal from the walk cap instead of the relay's 502.
6. **At X** the source changes from the board pass to the paper prep. Nothing visible.
7. **A reorg past H**: facts pause until the follower repins (one run), and the page shows the reorg banner. A page loaded
   before it keeps its old proof and is refused new facts at the old height until reloaded.
8. **`zync-serve` restarts** stay as today (a few ms of refused connections per new height); with nothing to load,
   they are instant.

## 12. Decisions (the user, 16.11.2026)

1. **Catch-up:** facts come from the **board pass's** live stores with retention (§6), not a freeze at ~140,000.
2. **Retention at the tip:** W = 288 blocks (ESTIMATE ~7 GB), so facts survive any reorg the automatic path handles.
   `HAZYNC_TRIE_RETAIN_MAX` defaults to 288.
3. **Coin-log facts at the tip:** `/coin` and `/find` are dropped in tip mode (the trie coin fact answers the same
   question). Z6 is not planned.
4. **History-fact cap:** a history-fact walk is refused past ~100,000 nodes (Z2).
5. **Release:** Z1 ships in the same host release as REORG P1, so each prep run restarts once.

## 13. Built: Z1 (16.11.2026)

`libzync/src/trie_store.rs` (host only: every addition is `#[cfg(feature = "std")]`, so the WASM verifier compiles
the file as before), `prover/host/src/prep.rs` and `trie.rs` (one call each).

- **`trie/roots.log`**: `TrieStore::log_root` after every accepted apply (prep and the proving path's `trie_commit`):
  80 bytes, `height u32 | block hash 32 | root reference u64 | root hash 32 (the state's trie_root bytes) | gen u32`,
  appended, no fsync.
- **`trie/GEN`** (u32, synced): bumped at every open of the arena (before any write) and by `TrieStore::bump_gen` (REORG
  P3's rewind will call it). At open, `roots.log` is cut back to the flushed height (a torn last entry too), and
  `retain/` lists of blocks above it are dropped.
- **Retention**: each freed slot is tagged with the height of the block whose update freed it (the root of tag − 1 is
  the last that holds it; held in `pending` as runs, one entry per block, not per slot). At a flush,
  `release = max(min(PIN, P), P − HAZYNC_TRIE_RETAIN_MAX)` (default 288): slots tagged ≤ release go to the free list
  as before, the rest to `trie/retain/r_<tag>.bin` (8 bytes per slot, synced before the meta), and lists ≤ release
  come back into the free list (deleted after the meta). **No `trie/PIN`: everything is released, exactly as before
  Z1.** When the cap releases a pinned height, `trie/PIN.released` says `released_through <h> pin <p> height <P>` and
  prep logs `TRIE_PIN_RELEASED`.
- **Review of 16.11.2026.** A release is committed by `retain/RELEASE` (the hash of the meta that frees the lists'
  slots and the lists' names, synced before that meta's rename; an open deletes the lists only if the meta on disk is
  that one), so a crash between the rename and the unlinking frees nothing twice. A `PIN` that does not read is a pin
  at 0. Slots an UNDO frees are released at the next flush (only the orphaned roots had them; the rewind's GEN bump
  tells a reader first). The lists of the orphaned blocks' FORWARD applies hold nodes of older roots, a pinned one
  among them: a rewind's flush at F keeps them, renamed `r_<F>_<k>.bin` (held until the pin passes F; an open keeps
  them as tag F ≤ the flushed height). **`trie/INTACT`** (`intact_from <h> gen <g> flushed <P>`), written and synced
  BEFORE each meta, at every flush with or without a pin: h is the highest freeing height ever released to the free
  list, so the root of every height ≥ h is intact (a pin written late, lowered, or unreadable is reported as what
  really happened, not as what it asks for). **`roots.log` holds a height more than once** after a rewind (F logged
  again, in the new generation): the reader takes the NEWEST entry for a height.
- **Re-review of 16.11.2026.** A rewind's re-tagged lists are named `r_<F>_<k>_<gen>.bin` (GEN makes them unique: two
  rewinds to the same height no longer rename onto each other), and pending runs freed by blocks above the flushed
  height (an apply and an undo in one session) are written the same way. Every re-tag is journaled first in
  `retain/RETAG`; an open whose meta did not reach F renames them back and drops the new ones. A rewind's cut of
  `roots.log` is journaled in `trie/ROOTS_CUT` (synced before the meta) and applied at the commit, or at the next open
  if the meta reached its height; a crash before keeps the old branch's roots. A store with Z1's files but no `INTACT`
  takes the highest height it ever logged or held as released (never claims more than it knows). After a rewind with
  no pin, or a pin above F, INTACT over-states what was released (its high-water mark stays at the old branch's
  height): safe for a reader, which then serves fewer heights until the chain passes it. `retain/RELEASE` changed format
  at `e29e9d03` → `73e436d3` (list names instead of heights); no store ran `e29e9d03` (Z1 was never deployed).
- **Not in Z1:** the read-only reader (`open_read`, `open_at`), `zync-serve` tip mode, the follower (Z2-Z5).

**Tests** (`trie_store.rs`): a disk store pinned at 20 runs to 80 with a flush every 5 blocks and a reopen; the root
of 20 from `roots.log` still reads as the trie of 20, snapshot byte for byte; the same run WITHOUT the pin (the
positive control) reuses its slots and the root of 20 no longer reads as that trie. Removing the pin returns every held
slot at the next flush; a cap of 2 releases a pin at 82 at the flush of 90 and writes `PIN.released`; the live trie
stays exact throughout. `GEN` counts opens and bumps, entries carry their generation, and after a crash past the flush
at 10 the entries of 11..15 and a lost block's `retain/` list are gone at the reopen.
