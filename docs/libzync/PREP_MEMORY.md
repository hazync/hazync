# libzync prep: where the memory goes, and what to change

16.11.2026. Paper prep (`libzync-prep`, `LZ_ROOT=/srv/bulk/libzync-prep`, stores on `/var/lib/libzync-fast`) on
hazync-proof (62 GB RAM, 16 cores, no swap), near block 636,000. Code read on `origin/main` 797b777e (the v0.26.0 host
that restarts prep from genesis). Every number is labelled **MEASURED** (read on the box or from a local run) or
**ESTIMATE** (worked out from the code and the measured numbers, with the working shown).

## 0. Summary

- The anon heap is **three structures**: the trie record cache (18 GiB, a setting), and the trie's two free-slot lists
  (2.7 + 3.1 GiB, which grow with block size). Everything else is under 0.3 GiB. MEASURED from `/proc/<pid>/smaps`.
- At each save point the trie makes **two more full copies** of the free list (`DiskArena::durable`), and the coin
  log's index makes a 40-byte-per-coin copy of the live set while it rebuilds (`DiskIndex::grow`, every ~15,000 blocks).
- The page cache (27 GB) and the mmap'd trie pages (13–21 GB) are reclaimable, apart from dirty pages, which the box's
  own sysctl lets build up to 12–24 GB for up to an hour.
- With today's settings the anon peak at a save point is ESTIMATE **~38 GB at 650k, ~44 GB at 800k, ~50 GB at the tip**,
  on a 62 GB box where the other services peak at ~23 GB. **The from-genesis run will not reach the tip as configured.**
- Two environment changes, no code, no image change and no format change, bring the tip peak to ESTIMATE **~20 GB**:
  `HAZYNC_PREP_SAVE_EVERY=250` and `HAZYNC_TRIE_CACHE_MB=12288`. Host-only code fixes (§6) remove most of what is left.

## 1. What the 28.5 GB anon heap is

`/proc/1638630/smaps`, mappings over 100 MB, 16.11.2026 15:4x CET (MEASURED, block ~635,950, between save points):

| mapping | size | resident | what it is |
|---|---|---|---|
| anon | 18.00 GiB | 18.00 GiB | trie **record cache** (`trie_store.rs` `RecCache`): 2^28 slots × (64 B record + 8 B tag) |
| anon | 4.00 GiB | 3.08 GiB | trie **`pending`** (`DiskArena.pending: Vec<u64>`): slots freed since the last flush, ~413 M |
| anon | 2.74 GiB | 2.74 GiB | trie **`free`** (`DiskArena.free`): slots free at the last flush, exactly the meta's 367,515,234 × 8 B |
| anon | 0.13 GiB | 0.13 GiB | BIP30 tree (`coinbase-smt` `Smt`: its node `HashMap`, ~634k leaves) |
| `[heap]` | 0.11 GiB | 0.10 GiB | per-block JSON, transactions, small maps |
| `nodes.bin` (shared file map) | 16 TiB | 20.74 GiB | the trie arena's mapping: page cache, not anon |

Sum of the anon rows: 24.05 GiB = `RssAnon` 25,224,984 kB (24.06 GiB). After the 636,000 save point (15:48 CET) the
new `free` is 2.82 GiB (meta 3,025,958,841 B = 378.2 M slots), the old one is gone, and `pending` keeps its 3.08 GiB
resident (`Vec::clear` keeps the allocation): **the steady state is cache + one free list + the largest pending ever.**

The 28.5 GB (26.5 GiB) in the request is 2.5 GiB above this. The coin log's index was being rebuilt at that time
(`index.grow` written 15:36–15:38, renamed 15:44 CET; that interval's coin-log phase took 2,300 s against the usual
700–900 s), and `DiskIndex::grow` holds every live coin as `([u8; 32], u64)` = 40 B: 2.5 GiB is ~67 M live coins, the
UTXO count of mid-2020. ESTIMATE (the rebuild was not sampled while its vector was alive).

Structure by structure, how each scales:

| structure | code | per element | scales with | 636k | transient? |
|---|---|---|---|---|---|
| record cache | `trie_store.rs` `RecCache::new`, `HAZYNC_TRIE_CACHE_MB` (drop-in `cache.conf` = 20480) | 72 B per slot; slots = the largest power of two ≤ MB·2^20/72 | **nothing**: fixed by the setting (9,216–18,431 → 9 GiB; 18,432–36,863 → 18 GiB) | 18 GiB, all of it touched | no |
| trie `free` | `DiskArena::durable` (`self.free = all`); `put` pops it | 8 B per slot | **F**, the slots freed per save interval (below) | 2.8 GiB | no: popping never shrinks it |
| trie `pending` | `DiskArena::free` pushes; `durable` clears | 8 B | F (resident = the largest F so far) | 3.1 GiB | no |
| `all` + meta bytes `b` | `DiskArena::durable` | 8 B + 8 B | F | +5.6 GiB | **at every save point** (trie flush, ~100–300 s) |
| live-coin vector | `host.rs` `DiskIndex::grow` | 40 B per live coin | UTXO count | ~2.5 GiB | **during an index rebuild** (when used slots reach half: every ~cap/10 inserts, ~15,000 blocks at 636k) |
| BIP30 tree | `coinbase-smt` `Smt` (`BTreeMap` leaves + `HashMap` nodes to depth 20) | ~36 B + ~65 B | ~one leaf per block (634k leaves; `.smt` 22.8 MB) | 0.13 GiB + heap | no |
| per block | `prep_block`: block JSON as `serde_json::Value` (twice: read and annotated), `ptxs`, `outs`, `first` | | block size | < 0.1 GiB | per block |

Not in memory: the coin log's levels and index (pread/pwrite, page cache), its journal `pending.bin` (a file), leaf
bundles and state files (written and dropped). The deployed host (11-09 build) has the same structures: the record
cache and the arena are #675 (11-09), and nothing since (tracked flags 11-15, undo hash 11-14) keeps anything across
blocks. The reorg undo records on `feat/reorg-p1-p2` hold one block's record (~1 MB) at a time.

### F, the frees per save interval

The trie is copy-on-write: every update writes a new node for each node on its path and frees the old ones, and none
can be reused before the next flush (`trie_store.rs` header). So F = SAVE_EVERY × updates per block × nodes per path.

| interval | F (meta) | mean ins+outs per block (`getblockstats`, 20 blocks) | frees per (in + out) |
|---|---|---|---|
| 370,001–371,000 | 207.3 M (`meta.unsorted`, 11-08) | 4,407 | 47.0 |
| 634,001–635,000 | 367.5 M | 7,809 | 47.1 |

MEASURED: the ratio is ~47 at both heights (the path grows by log2 of the trie, ~2.5 levels between them, within the
sample's noise). F at other heights below uses 47 × the sampled ins+outs × SAVE_EVERY (ESTIMATE; a 20-block sample
per height, so ±20 %).

## 2. Is the mmap / page cache a problem?

Mostly no. MEASURED in the prep cgroup: `file` 26.8 GB, of which `inactive_file` 25.6 GB; `file_mapped` 6.0 GB (13.2 GB
in the request; the `nodes.bin` mapping was 20.7 GiB resident later); `file_dirty` 3.5–5.2 GB; `pagetables` 0.6 GB;
`slab_reclaimable` 1.5 GB. Clean pages, mapped or not, are reclaimed before anything is killed, and host_lz's `RSS`
(40.6 GB) counts the mapped trie pages: RSS overstates what prep holds.

What is not freely reclaimable:

- **Dirty pages.** `/etc/sysctl.d/90-libzync-prep.conf` (MEASURED; not in the repo) sets `dirty_background_bytes` 12 GiB,
  `dirty_bytes` 24 GiB and `dirty_expire_centisecs` 360000 (one hour): trie writes are held until the save point's
  msync, on purpose (11-08: prep spent half its time throttled on dirty pages). Up to 12–24 GB of the page cache can be
  dirty, and reclaim must write it back first. That shows as memory pressure, not as an OOM.
- **Page tables.** 0.6 GB for 293 GB of `nodes.bin` (~2 MB per GB ever touched; freed only on unmap). ESTIMATE ~1.5 GB at
  the tip (trie ~0.7 TB: 3.3 nodes per coin ever × ~3.3 B coins × 64 B).
- The record cache is the page cache's better half for the address trie's inner records (64 B each instead of a 4 KiB
  page), so it competes with the page cache for the same RAM.

The "Under memory pressure" lines (310 in 24 h, MEASURED) are all `systemd-journald` dropping its own caches when its
cgroup's PSI crosses the threshold. Box-wide PSI is low (`some avg300=0.11`). They cluster in bursts (05:30–05:47,
08:00–08:21, 12:41–12:46, 13:21–13:28, 14:11–14:24 CET) that do not line up with prep's save points (05:03, 06:31, 07:35,
08:56, 10:06, 11:18, 12:26, 13:59) or with hazync-headers-follow, which runs every 10 minutes (~3.5 min, 9.1 GB peak,
~1 h CPU per run). Cosmetic today; the real risk is an anon peak, below.

Other anon on the box (MEASURED cgroup `anon`): bitcoind 2.5 GB (`dbcache=4000`, so up to ~4.5 GB), libzync-fetch
4.9 GB (peak 8.5 GB including cache), hazync-headers-follow 9.1 GB while it runs, the coordinator 0.06 GB. Kernel ~3–4 GB.
**Worst case besides prep ≈ 23 GB + kernel.**

## 3. What prep will need (ESTIMATE)

Anon only (page cache comes on top and is what makes prep fast). Steady = cache + free (8F) + pending (8F); save point
= steady + 2 × 8F; index rebuild = steady + 40 B × live coins. UTXO count: 163.2 M at the tip (`gettxoutsetinfo`,
MEASURED), ~70 M at 650k, ~115 M at 800k and ~180 M at 850k (ESTIMATE, public history).

| height | ins+outs / block | 8F | steady | **save-point peak** | index rebuild peak |
|---|---|---|---|---|---|
| 636k (MEASURED) | 7,809 | 2.9–3.0 GB | 25.8 GB | ~31.6 GB (from the code) | ~28.5 GB |
| 650k | 12,196 | 4.6 GB | 28.9 GB | **38.0 GB** | 31.7 GB |
| 800k | 15,876 | 6.0 GB | 31.8 GB | **43.7 GB** | 36.4 GB |
| 850k | 17,649 | 6.6 GB | 33.1 GB | 46.4 GB | 40.3 GB |
| tip (970k sample) | 20,010 | 7.5 GB | 34.9 GB | **49.9 GB** | 41.4 GB |

(The pending list keeps its largest size ever, so a heavy stretch such as 650k keeps its 8F resident afterwards.)

Against 62 GB: at the tip a save point alone leaves ~5 GB for bitcoind, the fetcher and the kernel, so it must OOM or
thrash. At 800k the margin with headers-follow running is negative. At 650k it is ~2–3 GB. **Headroom runs out between
650k and 800k.** On the last run's rates (437k → 635k in ~7 days, slowing), that is ESTIMATE ~10 and ~18 days after
tonight's genesis start: there is time, but the change is due before ~650k.

With the fixes (same method):

| configuration | 650k save peak | tip save peak | tip rebuild peak |
|---|---|---|---|
| today: 18 GiB cache, SAVE_EVERY 1000 | 38.0 | 49.9 | 41.4 |
| SAVE_EVERY 250 | 24.3 | 27.4 | 30.1 |
| SAVE_EVERY 250 + cache 9 GiB | 14.7 | 17.7 | 20.4 |
| + code fixes B, C, F (§6) | 11.2 | 12.0 | 12.0 |
| code fixes B, C, E, F only, cache 18 GiB, SAVE_EVERY 1000 | 22.0 | 23.6 | 23.6 |

## 4. A kill, the save points and the undo records

An OOM kill is SIGKILL, the same as the crashes the restart path already handles (review 11-12, tested by
`experimental/libzync-bench/prep/crash_test.sh`). A save point at h writes, in order: `state_h.json` (atomic), `.forest`,
`.flags`, `.smt` (atomic), the coin log (sync, then the `.coinlog` marker), the trie (`durable`: msync, fsync, meta by
rename, then the `.trie` marker), then deletes `pending.bin`. `libzync-prep-run.sh` takes the newest h with all of
`.json`, `.smt`, `.coinlog` and (`.trie` or the trie's meta at h). A kill:

- before the trie's meta rename: the restart takes the previous save point s; the coin log is undone through
  `pending.bin`; the trie is at its last flush, s, because no slot freed since then was reused. Lost: one interval.
- after the meta rename: the restart takes h (the trie-height rule); nothing to undo.
- during a coin-log index rebuild: `index.grow` is a temp file (removed at the next rebuild); `index.bin` is old or new
  by rename, and the journal undo puts it back to the save point either way.

The reorg undo records (`feat/reorg-p1-p2`, not on main) follow the same rule: written per block by rename, synced at a
save point before its state file, and those above the restart's save point removed at start (`prune_at_start`) and
written again as prep re-applies the blocks. A kill costs only time. A future rewind (P3) is designed to be
idempotent with its `out/REWIND` marker.

**The danger is a kill loop, not a kill**: if `MemoryMax` sits below the save-point peak, prep dies at every save point,
restarts 30 s later, redoes the interval and dies again, for ever. The peak grows with height (§3), so a `MemoryMax`
safe at 650k is not safe at 800k.

## 5. Local measurement: how much of F could be reused at once

`experimental/libzync-bench/trie_store/free_sim` (MEASURED, laptop, `RESULT_free_sim.txt`): `TrieStore` on a counting
arena, random scripts and outpoints (50 % or 80 % address reuse), 1–3 M coins, half spends and half new coins per
interval. For every freed slot it asks whether the slot was written since the last flush. The flushed trie cannot
refer to such a slot, so it could be reused at once.

| updates per interval | log2 | frees per update | of which written since the flush | held back |
|---|---|---|---|---|
| 3,000 | 11.55 | 24.8 | 12.25 | 50 % |
| 12,000 | 13.55 | 24.8 | 14.22 | 43 % |
| 48,000 | 15.55 | 24.8 | 16.31 | 34 % |
| 192,000 | 17.55 | 24.9 | 18.45 | 26 % |

Writes since the flush per update = log2(updates per interval) + ~0.8, whatever the trie size or address reuse (1 M,
3 M coins; 50 %, 80 % reuse). These are the top levels, which every update in the interval rewrites. On mainnet
(ESTIMATE): ~50 frees per update, 7.3 M updates per interval at 636k (log2 22.8), so ~24 of the 50 are
since-the-flush writes: **about half of F need not be held**, ~55 % with SAVE_EVERY 250. Real address reuse is far more
skewed (hot addresses' coin tries are shared within an interval too), so the real held share is at most this.

## 6. Fixes, ranked by gain and risk

Nothing here changes the guest or any image: the guest compiles only `coinlog.rs`, `coinlog_forest.rs`, `ttree.rs`,
`wire.rs`, `net.rs` from libzync and `roots.rs`, `bip30.rs` from coinbase-smt (`prover/methods/guest/src/main.rs`
`#[path]`; the build scripts include `profile.rs`). `trie_store.rs`, `host.rs`, `prep.rs`, `lz.rs` and `trie.rs` are host-only. No on-disk format changes.

| # | fix | kind | gain (tip, ESTIMATE) | risk / cost |
|---|---|---|---|---|
| 1 | `HAZYNC_PREP_SAVE_EVERY=250` in `run.conf` | env | save peak 49.9 → 27.4 GB; free lists ÷4 (−22.5 GB); a kill loses ≤ 250 blocks | Saves take 100–213 s per 1,000 blocks at 634–636k (MEASURED `PREP_TIMING save=`), 2.5–4 % of the time. Four times as many saves rewrite the hot pages more often: ESTIMATE +3–8 % wall, check `save=`. `LZ_WINDOW` must stay above it (it does). The trie's slot layout depends on the flush cadence, so store bytes compare only between runs with the same value |
| 2 | `HAZYNC_TRIE_CACHE_MB=12288` (2^27 slots, 9 GiB) instead of 20480 (18 GiB) | env | −9.66 GB at every height | Trie phase somewhat slower: halving drops roughly one more trie level per update out of the cache, ~1–2 of ~35 misses per update (MEASURED misses 291.6 M per 7.8 M updates at 636k), and the freed RAM goes to the page cache. ESTIMATE a few %. Compare `PREP_TIMING trie=` and the hit rate over 2–3 intervals past ~420k, where the trie outgrows RAM. Below that the trie fits in the page cache and the cache size changes little |
| 3 | `OOMScoreAdjust=500` on `libzync-prep` | unit | a global OOM kills prep, which is crash-safe (§4), not bitcoind, the coordinator or journald | none |
| 4 | **B + C** in `DiskArena` (`libzync/src/trie_store.rs`): in `durable`, take `pending`, append the left-over `free`, drop the old `free` before sorting, sort in place and stream the meta through a `BufWriter` instead of building `all` and `b`; in `put`, `shrink_to` the `free` list as it drains | host code | removes the two save-point copies and the drained free list's pages: the free lists cost ~1 × 8F at any time instead of 2–4 × 8F (tip, SAVE_EVERY 1000: −22.6 GB at a save point, −7.5 GB between them; with 250, a quarter of that) | Low. ~30 lines; the meta bytes stay identical (test: the same run's `meta` byte-identical before and after); on a write error, keep every slot as pending (never reusable early). Ships with the next host build, not tonight's |
| 5 | **F** in `DiskIndex::grow` (`libzync/src/host.rs`): two passes over the old file in 1 MiB reads (count the live coins for the new size, then insert), no `live` vector | host code | −40 B × live coins at each rebuild (2.5 GiB now, ~6.5 GB at the tip); the rebuild also gets faster (it read 2^28 slots by 40-byte `pread`; the 635–636k interval's coin-log phase took ~1,400 s more than usual, most likely the rebuild) | Low. Same insert order, so `index.bin` is byte-identical; test against the current code on one store |
| 6 | **E**: a slot written since the last flush goes straight back to `free` when it is freed (a bitset of written slots, ~0.55 GB at 4.4 B slots, cleared after the meta rename), not to `pending` | host code | F held ×~0.5 (§5); a smaller meta per save; fewer dirty pages, since hot slots are rewritten in place | Medium: it touches the invariant that makes a kill safe ("never overwrite a slot the flushed trie refers to"). A slot written since the flush is either past the meta's length or taken from its free list, so the flushed trie cannot refer to it; the bitset must be cleared only after the rename succeeds. Needs `crash_test.sh`, a differential run against `MemArena` (roots per block) and a review round |
| 7 | `MemoryHigh` / `MemoryMax` (§7) | unit | contains a runaway | only after 1+2 or 4; never below the peak |
| 8 | hazync-headers-follow: an `ExecCondition=` that skips a run while prep is within ~50 blocks of a save point or in one (prep's height from `out/tip` once reorg P1 lands; until then the newest `state_*.json`) | unit + script | keeps its 9.1 GB off the save-point spike | Small, and worth less once 1 or 4 is in (the spike shrinks to ~2 × 1.9 GB) |
| 9 | a 128 GB box | hardware | removes the limit; more trie in page cache, so faster | cost not checked |
| 10 | dirty-page sysctl (`90-libzync-prep.conf`): put it in the repo; lower only if pressure stalls matter | box | ≤ 12–24 GB of dirty cache | lowering it costs prep speed (11-08) |

Recommended for tonight's start, env only: **1 and 3**, and **2 unless a speed check past ~420k says otherwise** (the
cache can be raised again at any save point; a restart right after a `PREP_PROGRESS` line loses nothing). Then 4 and 5
in the next host release, and 6 after review. The board prep (`libzync-board-prep`, 10G/14G, SAVE_EVERY 1000, no cache)
has the same structures: at 1000 its save-point peak (~4 × 8F) passes 14 GB around 640–650k and it would kill-loop
there (and `MemoryHigh=10G` throttles it before that). It is at block ~6,000 today, gated by the provers. With SAVE_EVERY 250 it needs `MemoryHigh` ≥ 12G for the
index rebuilds near the tip.

## 7. systemd values

`MemoryHigh` throttles a cgroup that goes over it (reclaim and sleep), and `MemoryMax` kills it. Prep's anon cannot be
swapped (no swap), so both act on its page cache first, and that page cache is prep's speed. On the box today both are
`infinity` (`/etc/systemd/system.control/libzync-prep.service.d/50-Memory*.conf`, from `systemctl set-property`). That
overrides the repo unit's `MemoryHigh=10G` / `MemoryMax=14G`, which would kill-loop the paper prep at once (the cache
alone is 19.3 GB). The `cache.conf` drop-in and the sysctl file are not in the repo either.

- **Today's settings** (18 GiB cache, SAVE_EVERY 1000): leave both at `infinity`. A `MemoryMax` safe to the tip would be
  ≥ ~52G and would leave prep almost no page cache at its peaks; anything lower kill-loops somewhere past 650k.
- **With 1 + 2** (anon ≤ ~20 GB to the tip): `MemoryHigh=38G`, `MemoryMax=42G`. That leaves prep ~20–26 GB of page
  cache, about what it has today, and the other services 20 GB plus everything prep's cache gives back. The margin
  to a kill loop is 2× the peak.
- **With 1 only**: `MemoryHigh=46G`, `MemoryMax=50G` (peak ~30 GB including a rebuild).
- In every case `OOMScoreAdjust=500`. Do not turn on systemd-oomd for this unit: it kills on pressure, and prep runs
  under pressure by design.
