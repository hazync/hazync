# Trusted State: zAPPs before the backfill reaches the tip (proof of concept, planned for v0.28)

Status: **EXPERIMENTAL proof of concept**, branch `feat/trusted-state` (started 19.11.2026), laptop CPU only, under review.
Code: [`experimental/trusted-state/`](../../experimental/trusted-state/) and the host's `trusted-state` cargo feature
(`prover/host/src/trusted_state.rs`, off in every release and CI host). **No guest change; the core image ids do not
move** (§7). Every number below is MEASURED unless it says ESTIMATED, and says what it was measured on.

## 1. The idea in one paragraph

A zAPP answer is trustless only on a chain proved from genesis, and the backfill is far behind the tip. Bitcoin Core
pins, in its chainparams, the UTXO set at a few heights (`m_assumeutxo_data`: height, block hash,
`hash_serialized_3`). Take the newest such height H, build a core `ChainState` at H from Core's own `dumptxoutset`
(refused unless its `hash_serialized_3` is the pinned one), and prove blocks H+1..tip on top of it with the **unchanged
core image**, as a second lineage. zAPPs then work at the tip now, labelled "assumes Core's UTXO set at H". When the
main (genesis) lineage reaches H, its committed state must equal the anchored one in every field a later block's
verdict reads, `live_hash` (the ECMH of the live coins) first: that **join check** discharges the assumption.

Why a second lineage and not a shortcut into the main one: the coin log (COINLOG.md) holds every coin ever created in
chain order and the counting trie keeps spent history, so neither can be rebuilt from a snapshot. Only `live_hash`,
`utxo_value`, the tip and the time context are history-free. The anchored lineage therefore has its own coin log, trie
and BIP30 tree, built from the snapshot's coins, and its own `anchor_id`.

## 2. What answers each question of the PoC (summary)

| question | answer | measured on |
|---|---|---|
| 1. anchored `ChainState` from a Core snapshot, `hash_serialized_3` checked, block bound to headers, coin log + trie + live_hash built | **yes** (`ts-anchor build`) | regtest 101/104/105/110 (Core 28.0 dumps of the zAPP harness), mainnet 200 (Core 31.1.0 dump) |
| 2. anchored `live_hash` = main lineage's at H | **yes, every case**, against REAL receipts: regtest 104/105/110 (`eb2cd424`, GPU), mainnet 200 (`8888d59c`, GPU) | §5.2 |
| 3. guest takes the anchored state as base and validates further blocks | **yes, with no guest change**: `is_base = 1` (the existing base step); one host hook for the BIP30 tree | regtest anchored at 104 (blocks 105..112, incl. the BIP30 repeat at 105) and at 101 (102..112); mainnet anchored at 199 (block 200); executor (dev mode); every resulting state = the real receipts' |
| 3b. image id | **unchanged** (no guest source touched; laptop mainnet image rebuilt from clean: same id and ELF) | §7 |
| 4. cost | ingest in a guest: §6; per block anchored vs main: §5.4 | executor, laptop |
| 5. zAPP on the anchored state | balance statement: **identical answer** to the main lineage's; script history: **only from H** | §5.5 |
| 6. merge the lineages, nothing above H re-proved (second step) | yes, by **split answers + a join receipt**; the anchor MUST be proved (a forged coin log joins and its fake coin is spent: measured); replay pass ≈ 54% of re-proving (assumevalid proxy, measured on block 962,000) | §11 |

## 3. Design

### 3.1 The anchor (`ts-anchor build`)

Inputs: Core's `dumptxoutset` at H; the chainparams triple (H, block hash, `hash_serialized_3`) it must match; and three
side inputs a snapshot does not carry, written by `experimental/trusted-state/side_inputs.py` from the blocks (the
publisher runs it once; a node loading the anchor does not):

| side input | why the snapshot lacks it | used for |
|---|---|---|
| headers 0..H (80 B each) | a snapshot names only its base block | each coin's `creation_mtp` (in its leaf), `cum_work`, `recent_times` / `prev_time` / `prev_nbits` / `epoch_start` / `max_time`, the block forest's hashes. Bound to the pinned block hash by SHA-256d alone |
| BIP30-tracked live outpoints | Core does not track them (leaf flag bit 1, UTREEXO_FORMAT.md §2a) | the coin leaves (so `live_hash`) and the BIP30 tree |
| REPEATABLE txids (tracked transactions with no live output) | they have no coin in the set | the BIP30 tree: a later repeat of one is itself tracked (`tracked.rs` `coinbase_flags`) |

Output, the anchor at H, in the host's own formats (`<out>.json` a `ChainState`, `.coinlog`, `.trie`, `.smt`):

| field | value | from |
|---|---|---|
| `tip_hash`, `height` | the pinned block, H | chainparams = the headers' tip = the snapshot's base block |
| `utxo_roots`, `utxo_leaves` | a coin log of the snapshot's coins, **all unspent**, in the snapshot's order (Core's key order) | the snapshot |
| `utxo_value` | their sum | the snapshot |
| `live_set`, `live_hash` | the ECMH of their leaves | the snapshot + headers (MTP) + tracked list; zapp-assume's code |
| `trie_root` | the counting trie with every live coin inserted, **nothing spent** | the snapshot |
| `smt_root` | the BIP30 tree: every live coinbase and every tracked transaction with its count of live outputs; REPEATABLE slots | the snapshot + tracked + repeatable lists |
| `cum_work`, `prev_nbits`, `prev_time`, `epoch_start`, `recent_times`, `max_time` | as the main lineage computes them | the headers |
| `block_roots`, `block_leaves` | H+1 leaves (0x02 format): blocks 0..H-1 with their real hashes and the coin state **UNKNOWN** (a fixed digest no coin-state preimage hashes to), block H with the anchor's coin state | the headers |
| `profile` | the image's product profile (0x71 mainnet, 0x79 regtest) | the network |
| `self_id`, `anchor_id`, `lineage` | zero (as the genesis state) | — |

The anchored lineage's `anchor_id` is the guest's link digest of this state (SHA-256d of its journal words with
`anchor_id` zeroed), exactly as for any base step. That one value is the anchor's identity: a verifier pins it per
network and anchor height, as it pins the genesis anchor today.

The build is deterministic (the same inputs gave the same ANCHOR_ID on every repeated run) and refuses: a `hash_serialized_3`
or block other than the expected, another network's magic, headers that do not link, tracked outpoints not in the
snapshot, a REPEATABLE txid with live coins, and two coins with one trie key. Its single pass is cross-checked against
zapp-assume's reviewed `hash_snapshot` (coins, value, `hash_serialized_3`, ECMH must all agree).

### 3.2 The anchored lineage

**No guest change.** The core aggregate already has a base step, `is_base = 1`: it takes the previous state from the
input (not from a verified receipt), checks the carried ECMH point is on the curve, requires `block_leaves == height + 1`,
and commits `anchor_id = link_digest(prev)`. Every later block is ordinary recursion. That is exactly an anchored
lineage. The host change (`trusted-state` feature) is one hook: the BIP30 tree of a non-genesis anchor is otherwise
replayed from block files 1..H (a snapshot node has none) or, for a demonstration anchor, seeded from the range's
spends with the anchor's `smt_root` REWRITTEN, which changes its link digest. `HAZYNC_TS_SMT_SEED=<anchor>.smt` builds
the tree from the anchor's own list and requires its root to be the anchor's `smt_root`.

### 3.3 The BIP30 tree on the anchored lineage (assessed)

The main lineage's tree holds **every coinbase ever mined** (an untracked coinbase is never decremented, so it stays
"live" for ever) plus the tracked transactions. That set is history: a snapshot cannot rebuild it, so the anchored
`smt_root` differs from the main one by construction. What matters is whether any later block gets another verdict:

- A new coinbase is refused when its txid's slot is live. Main: every earlier coinbase. Anchored: every coinbase with a
  live output (that IS Core's BIP30 rule). They differ only for a repeat of a fully spent, untracked coinbase, which
  main refuses and Core and the anchored lineage accept. After BIP34 that repeat cannot happen at all (its scriptSig
  would have to start with the new height, so it would have been tracked), so **for an H above BIP34 (mainnet 227,931)
  the verdicts are identical**. Below BIP34 (regtest 104 here) the anchored lineage is Core-exact and main is the
  stricter one: a liveness divergence (main stalls on a block Core accepts), never a soundness one.
- Spends decrement tracked slots only, on both lineages, so they agree once the tracked bits agree.
- A repeat of a REPEATABLE tracked coinbase is itself tracked (leaf flag bit 1). Without the REPEATABLE slot in the
  anchor the repeat would be untracked and its leaf, so `live_hash`, would differ (control in §5.3).
- On mainnet no tracked coinbase can repeat before height 1,983,702 (the first height an old coinbase indicates; Core
  re-enables BIP30 there), so until then the tracked bits change no verdict at all; they change leaf bytes, which the
  join check compares.

## 4. Trust label and the join rule

**Before the join** (what a verifier prints for a proof whose `anchor_id` is a pinned Trusted State anchor):

> TRUSTED STATE: blocks H+1..T are valid under Bitcoin Core v28.0's rules (image M), **assuming** the UTXO set at
> block H (`<hash>`) is the one Bitcoin Core pins in its chainparams (`hash_serialized_3` `<hash3>`), and the
> anchor's side inputs (headers 0..H, the BIP30-tracked coins and REPEATABLE transactions). Not yet joined to a proof
> from genesis.

Every anchored proof is today a DEMO verdict (its `anchor_id` is not the genesis anchor); the label needs a pinned
table `(network, H, block, hash_serialized_3, anchor_id)` in the verifiers, as `genesis_anchor_hex` is now, and a
new verdict between OK and DEMO. Not built here.

**The join rule** (`ts-anchor join`): given the anchor A (its JSON), a receipt R_H of the main lineage at H that
verifies under a published image with the network's genesis `anchor_id`, and an anchored tip proof R_T:

0. each proof's `self_id` is the image it verifies under (R_H's = the main image, R_T's = the tip image): the core
   verifies its previous state under the `self_id` it is given, so without this a real-image receipt can carry a
   state a rogue image committed (§11.3 step 2; `--image` / `--tip-image`);
1. `R_T.anchor_id == link_digest(A)` (the anchored proof rests on exactly A);
2. equal in A and R_H: `kind`, `version`, `tip_hash`, `height`, `utxo_value`, `cum_work`, `prev_nbits`, `prev_time`,
   `epoch_start`, `recent_times`, `max_time`, `live_set`, `live_hash`, `profile`, `block_leaves` (every field a later
   block's verdict reads, except the BIP30 tree, §3.3);
3. NOT compared, each lineage's own: `utxo_roots` / `utxo_leaves` (the logs differ: all coins ever vs live coins),
   `trie_root` (history vs none), `smt_root` (§3.3), `block_roots` (UNKNOWN states below H), `self_id`, `lineage`,
   `anchor_id`.

Why that suffices: the anchored log's leaves are exactly A's coins **only if A is shown to be the deterministic
function of the snapshot** (the ingest receipt of §6, or the verifier's own rebuild; a forged A passes rules 1-2,
§11.1), and `live_hash` equality makes them, as a multiset of
leaves, the main lineage's live coins at H (ECMH collision resistance). Each leaf carries value, script, height,
coinbase and tracked flags and creation MTP, so every later spend, maturity and BIP68 check sees the same coin on both
lineages. After a successful join, R_T is as strong as a genesis proof of 0..T for **validity**; it still cannot answer
history questions below H (§5.5).

## 5. Measured on regtest and mainnet (laptop, WSL2, 19.11.2026)

Hosts: laptop builds of the core image from `261f0a59` in this worktree (`HAZYNC_BASE` = Core v28.0 + libsecp 0.5.1 +
patches, `scripts/libzync-base.sh`): regtest `a4344798…`, mainnet `420a7239…` (laptop ids; the published ids come from
the container build). Dev mode: every guest executed and its claims checked, nothing proved;
`HAZYNC_SEG_PO2=21 HAZYNC_CHUNKS=1 HAZYNC_PROFILE_LAYOUT=cost` (the harness's GPU settings). Run:
`experimental/trusted-state/run_regtest.sh`.

### 5.1 Anchors built (`ts-anchor build`, native, laptop)

| anchor | snapshot | coins | value (sat) | `hash_serialized_3` = expected | tracked / REPEATABLE | BIP30 entries | ANCHOR_ID (link digest) | build time |
|---|---|---|---|---|---|---|---|---|
| regtest 101 | Core 28.0 `utxo_101.dat` | 101 | 505,000,000,000 | `6d91814b…4f01` yes | 1 / 0 | 101 | `6e500817…9b5b` | 0.010 s |
| regtest 104 | `utxo_104.dat` | 169 | 519,999,999,000 | `97aa0565…9015` yes | 0 / 1 | 101 | `8fce9ea6…f71e` | 0.012 s |
| regtest 105 | `utxo_105.dat` | 170 | 524,999,999,000 | `00c942cb…802d` yes | 1 / 0 | 101 | `278379ec…d455` | 0.012 s |
| regtest 110 | `utxo_110.dat` | 177 | 549,999,999,000 | `c5016f0b…704d` yes | 1 / 0 | 102 | `ade95c90…afc8` | 0.013 s |
| mainnet 200 | Core 31.1.0 `zapps/assume/vectors/utxo_200.dat` | 204 | 1,000,000,000,000 | `a51f51ae…9d67` yes | 0 / 0 | 199 | `f825871a…bd25` | 0.038 s |

The expected triple for regtest is Core's own `gettxoutsetinfo` of the harness node (`oracle/snap_<H>.json`), the
stand-in for chainparams (Core's regtest `m_assumeutxo_data` is for its test framework's chain, not this one); for
mainnet 200 it is the value ASSUME_ZAPPS.md §5 recorded from Core 31.1.0. Repeated builds (101, 104, 110, mainnet 200,
each two or more times) gave the same ANCHOR_ID.

### 5.2 Join check (question 2): anchored state vs a REAL from-genesis proof at the same H

`ts-anchor join --main <receipt> --image <id>`: the receipt is verified first (risc0 3.1.0 verifier, dev mode compiled
out), then decoded, and its `self_id` must be `<id>` (§4 rule 0; added 21.11.2026, after the runs in this table; the
real `block_104.rcpt` passes it, re-checked 21.11.2026).

| H | main lineage receipt | receipt verifies | genesis anchor | all 15 JOIN fields equal (incl. `live_hash`, `utxo_value`, `cum_work`, time context) | lineage fields that differ |
|---|---|---|---|---|---|
| regtest 104 | `scripts/zapp-regtest/receipts/a/block_104.rcpt` (`eb2cd424`, one RTX PRO 6000, 16.11) | yes | `99509fc5…` | **yes** | `utxo_roots`/`utxo_leaves` (169 vs 173), `trie_root`, `smt_root`, `block_roots`, `anchor_id`, `self_id` |
| regtest 105 | `…/block_105.rcpt` | yes | yes | **yes** | same |
| regtest 110 | `…/block_110.rcpt` | yes | yes | **yes** | same |
| mainnet 200 | `swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt` (`8888d59c`, 11.11) | yes | `852c5c7e…` | **yes** | same |

### 5.3 The anchored lineage executes (question 3), and stays equal to the main one

`run_regtest.sh` with H = 104 (`TS_REGTEST_OK`, 6 min 52 s wall in all):

- **Anchored lineage, blocks 105..112 accepted** (main 105..110 and branch a 111..112), block 105 a base step on the
  anchor, 106..112 recursive; the directory held no block file below 105. The tip's `anchor_id` is ts-anchor's
  `8fce9ea6…`. Block 105 is the hard one: the byte-for-byte repeat of block 2's tracked coinbase (fully spent at 102,
  REPEATABLE in the anchor), a real 64-byte transaction and two P2PK spends of coins from before H.
- **Main lineage over the same blocks** from its own state at 104 (`receipts/a/state_104.*`, the real receipt's
  state, coin log and trie; BIP30 tree replayed from blocks 1..104): accepted.
- At 105, 110 and 112: `tip_hash`, `utxo_value`, `live_set`/`live_hash`, `cum_work`, `recent_times`, `max_time` equal
  on both lineages **and** equal to the real GPU receipts' states (`receipts/a/state_{105,110,112}.json`).
- Controls: a wrong expected `hash_serialized_3` (last digit changed) is refused at build; an anchor at 105 with the
  repeat coinbase's tracked bit dropped (same Core hash, since Core does not hash the bit) fails the join on
  `live_hash`; an anchor at 104 **without the REPEATABLE slot** still joins at 104 (the slot is in no JOIN field), and
  block 105 is then accepted on it but its repeat coinbase is not tracked, so `live_hash` at 105 differs from the main
  lineage's: the divergence surfaces at the next join, never as a wrong acceptance. So the REPEATABLE list is a needed
  side input, and it is checked only by a later join (open item, §8).

**Regtest, anchored at 101, blocks 102..112** (a separate run: `HAZYNC_TS_SMT_SEED=anchor_101.smt host
libzync-prove-range 102 112 <blocks 102..112 only> anchor_101.json <out>`, 233 s): all accepted. Here the tracked coin
is LIVE in the anchor (block 2's coinbase, leaf bit 1 from the side list), and block 102 spends it in the deposit
transaction (the BIP30 slot goes to REPEATABLE on the anchored lineage itself), so block 105's repeat is decided by
state the anchored lineage built. States at 104, 105, 110 and 112 equal the real receipts' in `tip_hash`,
`utxo_value`, `live_set`/`live_hash`, `cum_work`, `recent_times`, `max_time`, `prev_nbits`, `epoch_start`,
`profile`, `block_leaves`; the tip's `anchor_id` is the anchor's `6e500817…`. (The main lineage's per-block cycles
for this range are not measured: the repo has no main-lineage state at 101.)

**Mainnet, anchored at 199, block 200** (mainnet host `420a7239…`, `trusted-state` feature, dev mode): the anchor from
Core 31.1.0's `zapps/assume/vectors/utxo_199.dat` (203 coins; built with `--no-expect`: no Core-printed
`hash_serialized_3` for 199 is recorded in the repo, so here the check is the outcome below), ANCHOR_ID
`174e6b10…1c49`; block 200 accepted as a base step on it with only `block_200.json` in its directory. Its state at
200 against the REAL proof `mainnet_1_200_8888d59c.rcpt` (verified first): **JOIN_OK**, all 15 fields equal
(`live_hash`, `utxo_value`, `cum_work`, time context, tip), the lineage fields differ as expected. The main lineage's
block 200 for comparison: `libzync-prep 1 199` (native, 0.3 s) then the same block as a base step on prep's state.

### 5.4 Per-block cycles, anchored vs main (question 4b), same blocks, same image, executor

From the host's `LZ_STATS` lines (chunk + trie + aggregate receipts per block), `HAZYNC_SEG_PO2=21`, 1 chunk:

| block | anchored user cycles | main user cycles | difference | padded total (both) | segments (both) |
|---|---|---|---|---|---|
| 105 | 3,900,659 | 3,906,884 | −6,225 | 5,505,024 | 4 |
| 106 | 3,999,367 | 3,994,547 | +4,820 | 5,505,024 | 4 |
| 107 | 3,847,945 | 3,853,801 | −5,856 | 5,505,024 | 4 |
| 108 | 3,705,186 | 3,706,520 | −1,334 | 5,505,024 | 4 |
| 109 | 3,646,587 | 3,651,946 | −5,359 | 5,505,024 | 4 |
| 110 | 3,767,662 | 3,765,576 | +2,086 | 5,505,024 | 4 |
| 111 | 3,860,890 | 3,847,269 | +13,621 | 5,505,024 | 4 |
| 112 | 3,621,261 | 3,627,834 | −6,573 | 5,505,024 | 4 |
| **105..112** | **30,349,557** | **30,354,377** | **−4,820 (−0.02%)** | 44,040,192 | 32 |

Mainnet block 200 (coinbase only; trie + aggregate receipts): anchored 76,873 + 2,559,743 user cycles, main
76,873 + 2,544,619 (+15,124, +0.6%); padded 3,407,872 both. The difference is the coin log's shape: appending the
block's coinbase to the anchored log of 203 leaves merges two trees, to the main log of 208 leaves none (inferred
from the leaf counts, not profiled).

At this size the two lineages cost the same: the only structural differences are the coin log's depth (the anchored
log holds the live coins only, the main one every coin ever: 169 vs 173 leaves here) and the trie's size; each
changes a spend's witness by a few compressions. On mainnet the anchored log would hold ~N live coins against ~2.4
billion (COINLOG §8), about 4 fewer levels per spend path: a small saving, NOT measured.

### 5.5 zAPPs on the anchored lineage (question 5)

`ts-anchor facts` runs the zAPP libraries' own producer and checker (`zapp_statement::draft/sign/check_statement`,
`swiftsync_verify::make_trie_fact/check_trie_fact`) on the anchored tip state at 112 (H = 104) and, for comparison,
on the main lineage's REAL receipt state and trie at 112 (`receipts/a/state_112.*`). Receipt verification is not part
of this (the anchored receipts are dev-mode, and the shipped verifiers would call an anchored proof DEMO, §4).

| zAPP / fact | anchored at 112 | main at 112 | |
|---|---|---|---|
| balance statement, ALICE's six scripts, signed with her keys (BIP-322 / legacy) | STATEMENT_OK: 6 scripts, 28 unspent coins, 19,509,991,000 sat | identical | **works** |
| balance fact, 15 scripts (ALICE 6, MINER 6, CHANGE, DEPOSIT, `OP_1`) | 15/15 the same count and value as main | — | **works** (live coins only) |
| history fact, ALICE 6, MINER P2PK u / P2SH-P2WPKH / P2TR, DEPOSIT | equal (e.g. P2TR 17 paid / 2 spent both) | — | works when no coin of the script died at or before H |
| history fact, MINER P2PK c / P2PKH / P2WPKH | 18 paid / 1 spent; 18/1; 18/0 | 19/2; 19/2; 19/1 | **differs**: the coinbases of 1, 3, 4 spent at 102..104 are missing |
| history fact, CHANGE P2WPKH | 32 paid / 3 spent | 34 / 5 | **differs** |
| history fact, `OP_1` (`51`, the harness's anyone-can-spend) | 21 paid / 2 spent | 40 paid / 21 spent | **differs** (same-block chains at 103-104) |

Both sides' facts verify against their own trie roots and print "complete history". **On the anchored lineage that
wording is false**: the trie there starts at H with the live coins, so a history fact is "every coin paid to S that
was live at H or created after H", and an absence-style answer ("never paid") would be wrong for a script whose coins
all died before H. A Trusted State verifier must relabel history facts as "since H" (or refuse them) until the join;
balance, coin (UNSPENT/SPENT after H) and statement answers need no relabel beyond the §4 label. What cannot be
answered on the anchored lineage at all, joined or not: a coin's state at a height below H (the block forest commits
UNKNOWN there, so no coin path can open), spends before H, coins created and spent before H, and a script's complete
history. Transaction-in-block facts for blocks ≤ H should open (the leaf carries the real block hash), resting on the
header chain bound to the pinned block hash, i.e. on the assumption. These two follow from the anchor's construction;
neither was exercised here (no coin-path or tx fact was run on the anchored lineage).

## 6. Cost of building the anchor inside the zkVM (question 4a)

`experimental/trusted-state/ingest`: a separate measurement guest (its own image, laptop id `4e1ef3b6…`, built with
the host's stable Rust; nothing of it is in any core image). It reads the headers, Core's snapshot bytes and the hints
`ts-anchor build --ingest` writes, and computes in the zkVM: the header links and MTPs, Core's `hash_serialized_3`,
every coin's leaf, the ECMH (the core guest's own `swiftsync.rs` / `batch_ecdsa.rs`, `#[path]`-included, host-hinted
square roots), the coin log's roots (libzync `coinlog.rs`), the counting trie's root (libzync `ttree.rs`, every coin
inserted into the empty trie; the update order host-given and checked), the BIP30 tree's root (`coinbase-smt`
`roots.rs`, one proven insert per entry) and the block forest. Uncompressed P2PK keys (Core's special types 4/5, x
only) take y from the host, checked on the curve and of the stored parity. Not done in the guest: `cum_work` (one
256-bit division per header; the header-chain zAPP already proves chainwork).

**Every run's journal equals ts-anchor's native anchor field for field** (`INGEST_MATCH`: tip, value, coins,
`live_set`/`live_hash`, coin log roots, trie root, BIP30 root, block forest). Executor, default segment size (2^20),
laptop; `TSI` = the guest's own `env::cycle_count()` per phase:

| snapshot | coins | BIP30 entries | headers | snapshot phase (parse, hash3, leaf, ECMH, log, trie key) | trie | BIP30 tree | headers + forest | total user cycles | per coin, all phases | executor wall |
|---|---|---|---|---|---|---|---|---|---|---|
| regtest 101 (Core) | 101 | 101 | 102 | 729,098 (7,219/coin) | 108,371 (1,073/coin) | 147,267,013 | 479,264 | 148,601,458 | 1,471,302 | 2.9 s |
| regtest 104 (Core) | 169 | 101 | 105 | 1,193,217 (7,061/coin) | 181,069 (1,071/coin) | 147,266,995 | 493,409 | 149,153,100 | 882,563 | 3.0 s |
| regtest 110 (Core) | 177 | 102 | 111 | 1,255,805 (7,095/coin) | 190,281 (1,075/coin) | 148,720,820 | 518,947 | 150,704,786 | 851,439 | 3.0 s |
| mainnet 200 (Core 31.1.0) | 204 | 199 | 201 | 1,772,106 (8,687/coin) | 335,808 (1,646/coin) | 289,836,469 | 944,320 | 292,909,848 | 1,435,833 | 6.0 s |
| synthetic 1,000 | 1,000 | 0 | 111 | 6,423,875 (6,424/coin) | 2,266,449 (2,266/coin) | 353,747 | 519,452 | 9,589,398 | 9,589 | 0.2 s |
| synthetic 10,000 | 10,000 | 0 | 111 | 64,893,592 (6,489/coin) | 22,684,193 (2,268/coin) | 389,747 | 519,573 | 88,609,361 | 8,861 | 2.0 s |
| synthetic 100,000 | 100,000 | 0 | 111 | 649,717,864 (6,497/coin) | 226,812,714 (2,268/coin) | 749,747 | 519,991 | 878,888,034 | 8,789 | 21.2 s |
| synthetic 1,000,000 | 1,000,000 | 0 | 111 | 6,512,348,620 (6,512/coin) | 2,267,928,082 (2,268/coin) | 4,349,747 | 520,375 | 8,795,940,862 | 8,796 | 234.5 s (983 MB RSS) |

"synthetic" = `ts-anchor synth`: a Core-format snapshot of P2WPKH coins with random txids (2 per txid, none
coinbase, so no BIP30 entries), on the regtest headers to 110. NOT chain data; it measures the per-coin slope. The
BIP30 column at 0 entries is the empty-root setup plus the guest's scan of the txid groups for coinbase or tracked
ones (so it grows with the number of groups, 500,000 at 1 M coins).

What the table says:

- **Per coin: ~8.8 k cycles, linear from 1 k to 1 M coins** (6.5 k for the snapshot phase incl. the ECMH, 2.3 k for
  the trie insert). Real coins cost a little more in the snapshot phase where they are uncompressed P2PK (mainnet 200:
  8.7 k, y checked on the curve).
- **Per BIP30 entry: ~1.46 M cycles** (regtest 147.3 M / 101, mainnet 289.8 M / 199). That is the core's own SMT
  code: `roots.rs` `apply` = two `compute_root` walks of 256 levels, each recomputing the 256 empty-subtree hashes.
  It dominates every real snapshot here. Lever (NOT built, NOT measured): a bulk build of the sorted entries with the
  empty hashes computed once.
- **Per header: ~1.9 k (read, SHA-256d, link) + ~2.8 k (block forest leaf + merges)** cycles; the MTPs are in the
  snapshot phase, once per coin height.
- Native, for comparison (ts-anchor, one thread, laptop): 1 M coins 96 s wall, of which the ECMH hints for the guest
  (num-bigint) 78.8 s; 3.2 GB RSS (trie in memory).

**Scale limit.** One guest run held 1 M coins (983 MB host RSS in the executor). Mainnet's set is two orders of
magnitude larger, so a mainnet ingest must be a chain of chunk receipts carrying (hash3 SHA-256 midstate, ECMH
point, coin log roots, trie root, BIP30 root, count, value) from chunk to chunk, the trie part being exactly the
core's mode-17 trie update from an old root. Designed here, not built (§8).

## 7. The core image id does not move (question 3b)

No file the guest compiles is touched: `git diff origin/main...HEAD -- prover/methods libzync/src coinbase-smt
accumulator` is empty (the guest `#[path]`-includes from `libzync/src` and `coinbase-smt/src`). The host change is a
cargo feature of the host crate, which the guest build never sees.

Measured, mainnet image, laptop, same worktree path and base tree: the host built at `261f0a59` (origin/main, before
any change) gave `METHOD_ID 420a72393c50d431d81231180d22dc5dd7f165f1c15e5b01b80fd2763bcde2d6`, guest ELF sha256
`acbc59c1…5f57`. At `950a491` (all PoC code, host built `--features trusted-state`), after `rm -rf riscv-guest` and
`cargo clean -p methods` so the guest is compiled again from source: **the same METHOD_ID and the same ELF sha256**.
(Laptop ids differ from the published `46666b6f…` because ids depend on absolute build paths, LIBZYNC.md §8; the
comparison is like for like.) The regtest image (`a4344798…`) was not rebuilt from clean; its source is the same.

## 8. Open questions and risks (assessed, not solved here)

1. **Verifier support.** Every shipped verifier reports an anchored proof as DEMO (its `anchor_id` is not the genesis
   anchor). Trusted State needs, in `swiftsync-verify` and each zAPP's corecheck: a pinned table of anchors
   `(network, H, block, hash_serialized_3, anchor_id)` (built into the verifier like `genesis_anchor_hex`, so the
   anchor's identity is reproducible: anyone with Core's snapshot reruns `ts-anchor build` and must get the pinned
   `anchor_id`); a verdict between OK and DEMO with the §4 wording; the block forest read with 0x02 leaves for heights
   ≤ H (`undo_from` = H+1 for an anchored chain); and history facts relabelled "since H" (§5.5). **A prototype of the
   table and the verdict is §14.3** (a separate crate wrapping swiftsync-verify unmodified; the 965,000 entry PENDING).
2. **The side inputs are not covered by Core's hash.** `hash_serialized_3` covers outpoint, height, coinbase flag and
   output, not the tracked bit, the REPEATABLE slots or the creation MTP. The headers are bound by hashing; the
   tracked and REPEATABLE lists are checked only by a later join (a wrong one moves `live_hash`: §5.3 controls). On
   mainnet no tracked coinbase can repeat below 1,983,702, so before the join a wrong list changes leaf bytes, never a
   verdict; still, an anchor should not be pinned until the lists are reproduced independently (e.g.
   `assume_inputs.py`'s rule over the blocks by a second party), or until the join. Mainnet has 22 live tracked
   outputs at 974,111 (ASSUME_ZAPPS.md §3.2); the REPEATABLE count at 965,000 is not known here.
3. **The BIP30 tree argument (§3.3)** is reasoning over `bip30.rs`, `tracked.rs` and BIP34, checked on one regtest
   repeat (block 105) only. It needs a review of its own before an anchor below BIP34 is used anywhere, and a rig
   case: an anchored lineage below BIP34 fed a repeat of a fully spent untracked coinbase (Core and the anchored
   lineage accept, main refuses).
4. **Two lineages prove H+1..T twice.** The main lineage needs its own coin log and trie through those blocks for
   history zAPPs, so the anchored proofs are not reused by it after the join; the anchored lineage costs its blocks
   once more (§9). After a join it can either stop or keep serving balance-type zAPPs.
5. **A main proof at exactly H.** `live_hash` exists only at a proof's tip, so the join needs the main lineage's
   receipt AT H (the board's join tree must keep a root at H, or a linear proof through H). A board decision.
6. **Mainnet ingest in the zkVM** needs the chunked design of §6 and the BIP30 bulk-build lever; both unbuilt. ⛔
   **Corrected (§11.1): it is NOT optional.** The join compares `live_hash`, not the anchor's coin log, trie or BIP30
   tree, and the guest's base step does not tie them to `live_hash` either, so a natively built anchor with a forged
   coin log joins and lets its fake coin be spent (measured). Only a proof that the anchor's log, trie, BIP30 tree and
   `live_hash` come from one coin set (the ingest receipt), or a verifier that rebuilds the anchor itself from the
   snapshot, closes that.
7. **Host at mainnet scale.** The PoC keeps the anchor's coin log and trie in memory (1 M coins: 3.2 GB). A mainnet
   anchor needs the host's disk stores (`HAZYNC_COINLOG_DIR`, `HAZYNC_TRIE_DIR`) fed from the snapshot, and
   `HAZYNC_TS_SMT_SEED` refuses a resume (the PoC replays nothing): both small host changes, not made. **Built in §14.2**
   (22.11.2026): the disk stores fed by `ts-anchor build`, and a resume from `state_<k>.smt`.

## 9. Mainnet: the anchor height and an estimate path

**Anchor height: H = 965,000** (decided 21.11.2026). Core's mainnet `m_assumeutxo_data` entry for it is on master
and in v32.0 and later (added in Core commit `fed50258`, 10.09.2026; v32.1 released 28.10.2026): block
`00000000000000000001595977e6000ce56129f5c9b4073e31ccc30b90b97da9`, `hash_serialized_3`
`4a8d794337118c0c615b574f817c7306c687584a537184b8d233df42bf477ec2`, `m_chain_tx_count` 1,429,611,231. Earlier entries:
840,000, 880,000, 910,000 and 935,000 (block `0000000000000000000147034958af1652b2b91bba607beacc5e72a56f0fb5ee`,
`hash_serialized_3` `e4b90ef9eae834f56c4b64d2d50143cee10ad87994c614d7d04125e2a6025050`, the anchor this document
used until 21.11.2026; §12.4's header-times measurement and the first §13.4 run are at 935,000). The v28.0 tree the
guest is built from pins only 840,000. I expect the next entry with Core v33 (about April 2027); that is my inference
from Core's release cadence, not announced.

**The `--expect` string for 965,000:**
`965000:00000000000000000001595977e6000ce56129f5c9b4073e31ccc30b90b97da9:4a8d794337118c0c615b574f817c7306c687584a537184b8d233df42bf477ec2`.
Both hashes go in as Core prints them, with no byte reversal. `ts-anchor build` parses the block with
`from_display`, and it compares `hash_serialized_3` as a string against `display(SHA-256d)` (`zapp-assume`'s
`finish`), which is the order `gettxoutsetinfo` prints (`hashSerialized.GetHex()`). The regtest runs pass `--expect`
the RPC's `hash_serialized_3` and are accepted. Core's chainparams literals, `AssumeutxoHash{uint256{"…"}}` and
`uint256{"…"}`, are in the same `GetHex` order (the block hashes' leading zeros show it), and Core checks a loaded
snapshot by comparing that `uint256` with the same `hashSerialized`. So Core's chainparams strings are the `--expect`
strings as they stand.

**The real snapshot is not taken yet.** It needs `dumptxoutset` at 965,000 from a Core v32+ node. I have not run it,
and it waits for the user's OK. Until it exists, N (the live coins) and K (the BIP30 entries) at 965,000 are unknown,
and every mainnet figure below is per 10^8 coins and per 10^5 entries.

**What is ESTIMATED below** chains measured unit costs; nothing here was run at mainnet scale.

| step | unit cost (MEASURED) | mainnet quantity | estimate |
|---|---|---|---|
| native anchor build (`ts-anchor`, one thread, in memory) | 96 s and 3.2 GB per 10^6 coins (§6) | N live coins at 965,000: **not known here** (Core's `gettxoutsetinfo` gives it) | ~96 s × N/10^6 one thread before the disk stores; the ECMH part is parallel (ASSUME_ZAPPS.md §5: 1.9 µs/coin on 16 threads) |
| ingest receipt (**required**: §8.6 as corrected by §11.1; costed at scale in §12.5 and §13.4) | 8.8 k cycles/coin, 1.46 M cycles per BIP30 entry, 4.7 k cycles per header (§6, the one-shot guest) | N coins, K entries, 965,001 headers | 8.8×10^11 cycles per 10^8 coins + 1.46×10^11 per 10^5 entries + 4.5×10^9; at the header-chain zAPP's proven rate (2.35×10^9 user cycles in 59.4 min on one RTX PRO 6000, LIBZYNC.md §4, ≈ 0.66 M user cycles/s) ≈ 370 card-hours per 10^8 coins + 61 per 10^5 entries (user cycles: see §13.4's caveat on the padded ratio) |
| anchored lineage 965,001 → tip | per block = main's (§5.4); block 962,000 proved in 537 s on one RTX PRO 6000 (LIBZYNC.md §4) | 10,954 blocks to 975,954 (hazync-coord's tip, 15.11.2026; the tip has moved on since, at ~144 blocks a day) | ≈ 1,630 card-hours (10,954 × 537 s), ≈ $4.1 k at $2.49/hr, from that one block's time; real blocks vary (at H = 935,000 it was 40,954 blocks, ≈ 6,100 card-hours) |
| for scale: the main lineage from genesis | — | 975 k blocks | $278-295 k on PRO 6000 (WHOLE_CHAIN_COST.md, itself ESTIMATED, ±50%) |

So the anchored lineage is roughly 1.5% of a from-genesis proof's cost (ESTIMATED: $4.1 k against $278-295 k), paid a
second time for the blocks above H (§8.4), and it brings balance-type zAPPs to the tip once those ~11 k blocks (more by
the time it runs) are proved, under the §4 label, until the main lineage reaches 965,000 and the join discharges the
label.

## 10. Reproduce

```sh
scripts/libzync-base.sh <base>                                   # Core v28.0 + libsecp 0.5.1 + patches
cd prover && HAZYNC_BASE=<base> HAZYNC_LIBZYNC=1 HAZYNC_NETWORK=regtest cargo build --profile fasthost -p host --features trusted-state
cd experimental/trusted-state/anchor && cargo build --release      # ts-anchor (own workspace)
experimental/trusted-state/run_regtest.sh <host> <ts-anchor> <work dir> 104                     # §5.3, §5.4: TS_REGTEST_OK
ts-anchor join --anchor <work>/anchor_104.json --main scripts/zapp-regtest/receipts/a/block_104.rcpt --image eb2cd424…   # §5.2
ts-anchor facts --state <work>/out_anc/state_112.json --trie <work>/out_anc/state_112.trie \
                --scenarios scripts/zapp-regtest/chain/scenarios.json --who ALICE [--history <spk>]…                   # §5.5
python3 experimental/trusted-state/side_inputs.py prover/testdata/libzync/mainnet 200 <side>                          # mainnet 200
ts-anchor build --network mainnet --snapshot zapps/assume/vectors/utxo_200.dat --side <side> \
                --expect 200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320:a51f51ae…9d67 --out <a> --ingest <a>.ingest
cd experimental/trusted-state/ingest && cargo build --release -p ts-ingest-host; ts-ingest-host <a> <snapshot> <side>    # §6
# §11.1 forged anchor: build with --forge-extra 5000000000:51:50, then
python3 experimental/trusted-state/forge_block.py scripts/zapp-regtest/blocks/main/block_105.json <FORGED txid> 5000000000 50 <mtp> <dir>/block_105.json
HAZYNC_TS_SMT_SEED=<forged>.smt RISC0_DEV_MODE=1 host libzync-prove-range 105 105 <dir> <forged>.json <out>             # accepted
# §11.2 / §11.3
ts-anchor split-history --main-h scripts/zapp-regtest/receipts/a/state_104 --anchored-t <work>/out_anc/state_112 \
                        --main-t scripts/zapp-regtest/receipts/a/state_112 --scenarios scripts/zapp-regtest/chain/scenarios.json
ts-anchor join-receipt --main scripts/zapp-regtest/receipts/a/block_104.rcpt --main-image eb2cd424… \
                       --ingest-journal <a>.ingest.journal --anchored-tip <work>/out_anc/state_112.json --tip-image a4344798…
# §11.4: a mainnet host built with HAZYNC_LIBZYNC=assumevalid, block 962,000, both images:
HAZYNC_CHUNKS=16 HAZYNC_BLOCK=prover/block_962000.json host libzync-hints b.lzb
HAZYNC_CHUNKS=16 HAZYNC_BLOCK=prover/block_962000.json HAZYNC_LZ_HINTS=b.lzb \
  HAZYNC_COINLOG_TXCOUNTS=experimental/libzync-bench/cost/txcounts.txt host agg-exec-fake
python3 experimental/trusted-state/replay_share.py ~/libzync-gpu/e1/prof/*.pprof
```


## 11. Merging the two lineages: nothing above H re-proved (19.11.2026, second PoC step)

The question: once the main (genesis) lineage is proved to H and joined, can the anchored lineage's proofs of H+1..T
stand in for the main lineage's, so that nothing above H is proved twice? Validity and the live-coin state can carry
over (§11.1, with one hard condition); history cannot, because the main lineage's coin log and counting trie for H..T
are never built. The options are: answers split across the two proofs (§11.2, with a join receipt, §11.3), a cheaper
"replay" of the main lineage's state over H..T (§11.4), or a full re-prove.

### 11.1 The anchor must itself be proved (negative control, MEASURED)

**Reasoning over the guest.** On `is_base = 1` the aggregate (`prover/methods/guest/src/main.rs` `aggregate`) checks
of the input state only: `kind`/`version`, the carried ECMH point is a curve point (`swiftsync::start`),
`block_leaves == height + 1`, the block's height and parent hash, `w.root_prev == prev.utxo_roots` (the witness's log
is the state's), `w.in_smt_root == prev.smt_root`, and the trie receipt starts from `prev.trie_root`. Nothing relates
`utxo_roots`, `trie_root` or `smt_root` to `live_hash`; spends are opened against `utxo_roots` alone, and the ECMH
only subtracts. The join (§4) compares `live_hash` and never the log. So an anchor whose coin log holds a coin that is
not in its `live_hash` joins, and the coin can be spent above H. **Confirmed.**

**Control** (`ts-anchor build --forge-extra 5000000000:51:50`, `forge_block.py`): the honest regtest anchor at 104
plus ONE extra OP_TRUE coin of 50 BTC (height 50, txid `aeab1f0a…`) appended to its coin log and trie, `live_hash`,
`utxo_value` and `hash_serialized_3` left honest; and block 105 with one extra transaction spending it (merkle root
and nonce redone, so the header is valid; Core would reject the block).

| check | result |
|---|---|
| `ts-anchor join` of the forged anchor against the REAL receipt at 104 | **JOIN_OK** (all 15 fields equal; only `utxo_leaves` 170 vs 173 differs, a lineage field) |
| the core image (regtest `a4344798`, executor) on the forged block 105, base step on the forged anchor | **accepted**: chunk 1,294,909 + trie 74,142 + aggregate 2,580,445 user cycles, `LZ_PROVE_RANGE_DONE` |
| its state at 105 against the real receipt's | `live_hash` differs (the guest subtracted a point it never added), `utxo_value` equal |
| the ingest guest on the true snapshot, compared with the forged anchor | **INGEST_MISMATCH** `coins, trie_root, utxo_roots, block_roots` |
| the join-receipt prototype (§11.3) on the forged lineage's tip | **refused**: its `anchor_id` `5cf47301…` != the link digest rebuilt from the ingest journal `8fce9ea6…` |
| a split history (§11.2) of `OP_1` with the forged lineage's trie at 105 | passes the matching rule, but gives 41 coins against the main lineage's 39 |

So: the proven `live_hash` constrains a forged anchor's LIVE SET, not its coin log. After the join, the anchored
proofs above H are as good as the main lineage's **only if** the anchor is the output of the ingest computation
(receipt), or the verifier rebuilt it from the snapshot itself. §8.6 is corrected accordingly. (The same holds before
the join: an anchor nobody reproduced is a trust assumption beyond Core's snapshot.)

What the ingest receipt still takes on trust: the REPEATABLE list (§3.3). A wrong entry cannot make the anchored
lineage accept a block Core refuses (BIP30 forbids repeating an outpoint that is live; live coinbases and tracked
transactions come from the snapshot, and the ingest guest refuses a REPEATABLE txid with live coins), but it changes
the tracked bit of a later repeat, so the leaf bytes and `live_hash` drift from the main lineage's (§5.3 control). On
mainnet no repeat is possible below 1,983,702. A join receipt could check each listed entry against main@H's
`smt_root` (a membership proof of value REPEATABLE); completeness cannot be shown that way. Open.

### 11.2 Split answers, per zAPP

After a proven join at H (the join receipt, §11.3): main@H has the full history 0..H (coin log, trie, block forest);
anchored@T has the live set at H and the full history of H+1..T. A coin's identity across the two is its **leaf**
(SHA-256 over outpoint, value, script, height, flags, creation MTP) or its **trie key** (script hash ‖ coin key), both
computed from the coin's fields, so they are the same bytes on both lineages; positions in the two coin logs are each
lineage's own and are never matched. The join's `live_hash` equality plus the ingest receipt make the anchored trie's
and log's entries at H exactly main@H's live coins.

| zAPP / fact | answer at T from | matching needed | verdict |
|---|---|---|---|
| balance fact, balance statement (`zapps/statement`) | anchored@T alone | none: live coins only | **clean** (§5.5: identical to main@112) |
| assumeutxo (`zapps/assume`) | anchored@T `live_hash` alone | none | **clean** |
| assumevalid (`zapps/assume`) | X ≤ H: main@H; X > H: anchored@T + join receipt | the join receipt (one chain) | **clean** |
| headers (`zapps/headers`) | its own proof | none | **unaffected** |
| deposit (`zapps/deposit`, on main since #743) | tx block h ≤ H: main@H's block forest; h > H: anchored@T's (leaves above H carry the anchored state and the same undo hash); header chain as today | which proof covers h; both bound to one chain by the join receipt | **clean** (verifier change: two core proofs + join receipt) |
| trie coin fact (unspent / spent / absent) | main@H if SPENT by H (final); else anchored@T; ABSENT only if absent in both | the coin key, same bytes | **clean** |
| trie history fact | main@H's coins SPENT by H ∪ anchored@T's coins; a key in both (a coin re-created over its own spent record) keeps the anchored one, as the main trie does | every coin UNSPENT at H on main must be in anchored@T's history with its value and spent, if at all, above H | **clean, MEASURED**: `ts-anchor split-history`, main@104 (real receipt's trie) + anchored@112 → **15/15 scripts equal to the main lineage's history at 112, coin for coin**, incl. the BIP30 re-creation at 105 and `OP_1` (39+21 → 40 coins, 21 spent) |
| coin-log CoinFact (position) / liveness (`zapps/liveness`) | coin created ≤ H and spent ≤ H: main@H; otherwise anchored@T, the coin found by its leaf in the anchored log; `at` ≤ H from main@H's block forest, `at` > H from anchored@T's | the leaf, same bytes; positions never compared | **clean** (reasoned; not run) |
| CoinJourney | each hop's block from the proof covering its height | as deposit | **clean** (reasoned) |
| block forest: block h in chain, coin state at h | h ≤ H main@H (real states); h ≥ H anchored@T | none | **clean** |
| hints (`zapps/hints`), utreexo (`feat/zapp-utreexo`): walks from genesis that rebuild the coin log and block forest and require them equal to ONE core journal | — | the walk's coin log at T is the main lineage's, which no proof commits above H | **needs machinery**: a two-part walk (runs ≤ H bound to main@H as today; runs above H opening blocks and undo hashes in anchored@T's forest; the final binding at T by `live_hash` or by the forest alone). A zAPP image change, not a core one. Not built |
| "complete history" of anything below H on the anchored proof alone | — | — | **cannot**, without main@H |

Nothing in the table needs the two lineages' coin-log positions matched; that is what makes the split cheap. The cost
is that every history answer above H carries two facts and the join receipt, for as long as the main lineage stays
at H.

### 11.3 A join receipt

**What it binds.** A small separate guest (a zAPP-like image; no core change) would take three receipts, each
`env::verify`'d: main@H (a published core image, genesis-anchored), the ingest receipt of the anchor, and the anchored
lineage's tip T (the core image). It then:

1. rebuilds the anchor A: every coin-derived field from the ingest journal (`tip_hash`, `height`, `utxo_roots`,
   `utxo_leaves`, `utxo_value`, `live_set`/`live_hash`, `trie_root`, `smt_root`, `block_roots`, `block_leaves`) and
   every history-free header field **from main@H's journal** (`cum_work`, `prev_nbits`, `prev_time`, `epoch_start`,
   `recent_times`, `max_time`, `profile`); `self_id`, `anchor_id`, `lineage` zero;
2. requires the ingest's (H, tip, value, live set) = main@H's, main@H genesis-anchored, the same profile, and each
   core journal's `self_id` = the image its receipt was verified under (main@H's = `main_image`, J_T's =
   `tip_image`). The core aggregate verifies its previous state and its chunks under the `self_id` it is GIVEN and
   commits it; its soundness rests on the verifier checking the final `self_id` == the image id (S1,
   `prover/methods/guest/src/main.rs`; the host's `lz.rs` checks it on every resume). Without it, the real core image
   run with `self_id` = a rogue image X, which commits whatever state it likes, gives a genuine core receipt of a
   forged genesis-anchored main@H, or of a tip whose blocks were never checked, and the join would accept either;
3. requires J_T.`anchor_id` == `link_digest(A)`. Because A's header fields were taken from main@H, this one equality
   also proves the anchor's time context and work were main@H's: rule 2 of §4 for those fields comes for free;
4. commits the joined statement: both image ids, the genesis anchor, H and its block, T and its tip, chainwork and
   `max_time` of T, and the digests of J_H and J_T, so a split answer (§11.2) checks its main@H facts against J_H's
   roots and its anchored facts against J_T's.

**What the verifier pins.** The join receipt proves its statement only relative to the image ids it is given, and it
commits them: `main_image`, `tip_image`, `ingest_image` (the combine), `chunk_image`, `bip30_image` (zero for an
inline BIP30 tree) and `genesis` (the network's genesis anchor). A verifier must pin **all six** and refuse a joined
statement that names any other value; a join over images of the prover's choosing proves nothing. No pin file exists
yet: pinning is a release step, with the ids taken from reproducible-container builds (a laptop build's ids are not
pinnable; they move with the toolchain and paths). The ids printed by a `TS_CONTROLS=1` build must never be pinned:
those images read a cut mask and can skip any check, and the host labels every result line of such a build
`CONTROLS BUILD`.

It needs no block forest or handover machinery: the block forest of the anchor is already inside A (ingest), and the
link digest is the guest's own. HANDOVER.md's cross-lineage tools (`lineage`, `is_base = 2`) do not apply: a
handover continues ONE lineage under a new image, while here two lineages of the same image meet.

**Prototype, host side** (`ts-anchor join-receipt`; the checks of steps 1-3 natively, main@H's receipt really
verified, the ingest journal from the ingest guest's executor run, the anchored tip from the dev-mode run of §5.3):

| case | result |
|---|---|
| main@104 (real receipt `eb2cd424`) + ingest of `utxo_104.dat` + anchored tip at 112 | **JOINED**: the rebuilt anchor's link digest is `8fce9ea6…`, exactly the anchored lineage's `anchor_id`; commits J_H `4f2f807d…`, J_T `59e2e868…` |
| the same, with the forged lineage's tip (§11.1) | refused: `anchor_id` `5cf47301…` != `8fce9ea6…` |
| main@105 instead of 104 | refused: ingest (H, tip, value, live set) != main's, and the link digest moves (`fdfc8a7a…`) |

The rebuild in step 1 reproducing the anchored `anchor_id` bit for bit is the measured point: the ingest journal plus
main@H's header fields ARE the anchor. Not built as a guest; its cycles are not measured. Inside the guest it is three
journal decodes, one serde encoding and one SHA-256d; the cost that matters is resolving three receipt assumptions in
recursion, of the order of the core's own join (mode 18, 6.4 s on one RTX PRO 6000, LIBZYNC.md §4): ESTIMATED, not
measured.

### 11.4 The replay pass

**Idea.** Re-derive the main lineage's state over H+1..T (coin log flips and appends, counting trie, BIP30 tree, ECMH,
`utxo_value`, block forest with main's coin states and the undo hashes) from blocks the anchored lineage already
proved valid, skipping script and signature checks, so the main lineage reaches T with its full history and carries
on as the one lineage.

**Soundness (reasoned).** Skipping is sound for exactly the checks whose verdict the anchored proof already carries,
and only for the same blocks:
- the replayed blocks must be the anchored lineage's: each replay step links its header to the previous main state's
  `tip_hash`, and the replay's block at height h must open as leaf h of anchored@T's block forest (or the replay's tip
  at T must equal J_T's `tip_hash`, the header chain then fixing every block in between); the merkle root binds the
  transactions to the header;
- the anchored proofs are valid for the main lineage only through the join receipt (§11.3), which needs the ingest
  receipt (§11.1);
- everything that COMPUTES state must still run: the coin log flips open the spent leaf against main's own log (so the
  existence check is repeated, harmlessly), the trie update, the BIP30 transition (main's tree differs from the
  anchored one, §3.3), the ECMH, the undo hash. What may be skipped decides only validity: `VerifyScript`, sighash,
  signature parse and verify, the batch MSM, and (cheap) the block's weight, sigops, fee and subsidy sums.

**Guest work.** A replay step is the aggregate with chunks that only parse transactions and derive txids and coin
leaves, and an `env::verify` of the anchored proof (or one join receipt per range). Its receipt must be able to stand
as main's previous state for the core image's next ordinary block, which today's guest accepts only from its own
image (`is_base = 0`, `self_id`) or a pinned predecessor with the SAME profile (handover). So it is a **core image
change**: a new mode or a new image with its own lineage rule, i.e. a release and a review of new consensus-adjacent
code. The existing assumevalid build (`HAZYNC_LIBZYNC=assumevalid`, profile bit 1) is the nearest thing that exists,
but its proofs say "scripts skipped" and cannot be handed over to the scripts-checked image (HANDOVER.md §4 step 3).

**Measured basis for the saving.**

1. *The assumevalid build as a proxy*, block 962,000, executor, laptop, this branch's host (`950a491`+), 16 chunks,
   `agg-exec-fake` (modelled coin log around the real block, `cost/txcounts.txt`), mainnet laptop images
   `420a7239…` (scripts checked) and `fd04b41d…` (assumevalid):

   | | chunks (16) | aggregate | trie | **block** | padded (segments) |
   |---|---|---|---|---|---|
   | scripts checked | 385,230,206 | 66,539,127 | 47,599,783 | **499,369,116** | 609,239,040 (586) |
   | assumevalid (scripts and signatures skipped) | 153,191,036 | 66,539,127 | 47,599,783 | **267,329,946** | 330,989,568 (324) |
   | saved | −60.2% | 0 | 0 | **−46.5%** | −45.7% |

   The chunks still parse every transaction, hash txids and wtxids and build the coin leaves; aggregate and trie
   are unchanged. A purpose-built replay step would save at most about this much (it keeps the aggregate and the
   trie, 114 M of the 267 M) and pays its own binding to the anchored proof.
2. *The profiles* (`experimental/trusted-state/replay_share.py` on `~/libzync-gpu/e1/prof/*.pprof`, E1-era images, a
   sample charged as skippable when ANY frame of its stack is script or signature verification): one chunk of block
   962,000 48.1% skippable (74.8% if the guest crate's bare inlined frame, which core_share.py charges to the batch
   finish, is counted too), of 750,000 53.9% (85.6%); the aggregate and the trie receipt 0.0%. The measured 60.2% of
   (1) falls inside that range. These are larger than a self-time split of the same profiles (signatures ~21-30%,
   script ~6-10% of a chunk) because the paging, memcpy and hashing done for the interpreter and the batch are
   charged to them here.

So a replay of H..T costs, by this proxy, **about 54% of proving H..T again** (ESTIMATED for real blocks from one
measured block; 750,000's profile suggests a similar share). It removes no work from the trie, the BIP30 tree or the
coin log, which are what the main lineage needs.

### 11.5 Recommendation for v0.28

**Split answers with a join receipt; the anchored lineage stays the tip lineage after the join; the ingest receipt
is mandatory.** Reasons:

- It needs **no core image change**: the anchored lineage is the core's own base step (§3.2), the join receipt and the
  ingest receipt are separate small images, and split answers are verifier and zAPP-module work. A replay pass needs a
  new core mode or image (a release window and a review of new state-transition code) and still costs about 54% of
  re-proving H..T (§11.4); a full re-prove costs 100% (≈ 1,630 card-hours above 965,000 to the 15.11.2026 tip, §9, ESTIMATED).
- Validity and the live state carry over completely once the anchor is proved (§11.1); every live-state zAPP
  (statement, balances, assumeutxo, liveness and coin facts for coins live at H or created above it) answers from
  anchored@T alone, and the history zAPPs split cleanly with no position matching (§11.2; complete histories measured
  equal to the main lineage's on 15/15 scripts).
- The costs it leaves: two facts per history answer and a frozen main proof at H; the hints and utreexo walks need a
  two-part walk (a zAPP image change); the verifiers need the joined-statement format; the ingest at mainnet scale
  needs the chunked design and the BIP30 bulk-build lever (§6, §8.6). The ingest receipt is the critical path: without
  it neither the label before the join nor the merge after it is sound.
- Keep the replay pass as the later option if single-proof history at the tip becomes a requirement (one store for
  `zync-serve`, unchanged hints and utreexo walks); it is then a release item of its own.

## 12. The ingest receipt at scale (third PoC step, 19.11.2026)

§11.1 made the ingest receipt the critical path. Two things stood in the way at mainnet scale: the BIP30 tree cost
1.46 M cycles per entry, and one guest run holds about 10^6 coins. Both are addressed here in the **ingest images only**
(`experimental/trusted-state/ingest`); the core guest's sources are untouched (§12.5).

### 12.1 The BIP30 tree: a bulk build

**Cause.** The core's `roots.rs` `apply` (one proven insert per entry) runs `verify` and then a second `compute_root`,
each a 256-level walk, and each `compute_root` first rebuilds the 256 empty-subtree hashes: about 3 × 256 SHA-256s of
65 bytes per entry.

**Fix, ingest only** (`ingest/tsi.rs`, `smt_bulk`): the root of all K entries at once, sorted by key: a subtree with no
entry is the precomputed `empty[height]`, a subtree with one entry is that leaf folded up its remaining path against
`empty`, otherwise hash the two halves split on the next key bit. About K·(256 − log2 K) node hashes in all, the empty
table built once. The entries come sorted for free (Core's file order is txid order, which is the tree's key order);
the guest merges the REPEATABLE list in and requires keys strictly increasing (no key twice, so no REPEATABLE txid with
live coins). Three variants of the same tree, chosen by an input word: `bulk` (roots.rs's own `hash_leaf` /
`hash_node`), `raw` (the same two hashes as hand-padded `sha2::compress256` calls), `sys` (the same blocks through
risc0's SHA accelerator directly, as libzync `coinlog.rs` does).

**Equality with the core's tree.** Every run below ends `INGEST_MATCH`: the guest's root equals `ts-anchor build`'s,
which inserts the same entries into the core's own `hazync_coinbase_smt::Smt`.

| set | entries | proofs (roots.rs apply, before) | bulk | raw | **sys** | sys per entry |
|---|---|---|---|---|---|---|
| regtest 104 (Core) | 101 | 147,283,772 | 35,061,309 | 29,240,366 | **22,733,379** | 225,083 |
| regtest 110 (Core) | 102 | 148,737,758 | 35,337,573 | 29,462,100 | **22,893,210** | 224,443 |
| mainnet 200 (Core 31.1.0) | 199 | 289,869,509 | 68,285,043 | 56,959,279 | **44,260,268** | 222,413 |
| synthetic 20,000 coins, every 2nd txid a coinbase | 5,000 | 7,276,049,952 (1,455,210/entry) | 1,679,572,534 | 1,401,266,721 | **1,088,769,916** | **217,754** |

BIP30-phase user cycles (the guest's `env::cycle_count()`), executor, one-shot ingest guest. **1.455 M → 218 k cycles
per entry, 6.7×.** In the combine guest (§12.2), where the tree is built in a fresh process, the same 101 regtest
entries took 13,712,033 cycles (136 k per entry): the per-entry cost depends on paging, so the projection (§12.4) uses
the higher, one-shot figure.

### 12.2 Chunked ingest: design

Three images in `experimental/trusted-state/ingest` (measurement only, none in the core): the one-shot `ts-ingest`
(§6), and `ts-chunk` + `ts-combine`. The host side is `ts-anchor chunk-plan`; `ingest/tsi.rs` (the carried state, the
SHA-256 stream, the bulk tree) is one file `#[path]`-included by all three, so they cannot disagree on an encoding.

**Cut by file order, not key range.** `hash_serialized_3` is one SHA-256d over Core's file order, the coin log is
appended in that order, and the BIP30 entries arrive sorted in it; a key-range split would have to reorder all three.
So chunk i is a run of whole coins, in file order. A txid group may straddle two chunks.

**The carried state** (`tsi::ChunkState`, one canonical byte encoding, read back only if re-encoding gives the same
bytes): the file header (magic, base block, coin count), coins read, value; the open txid group (txid, coins still to
come, coins read, coinbase / tracked flags, last encoded vout, for Core's key order); the `hash_serialized_3` inner
SHA-256 **midstate** (8 state words, the < 64-byte unhashed tail, the length), which crosses a chunk boundary
unfinished; the ECMH partial sum (affine; the next chunk adds it in with `swiftsync::start`, which checks it is a
curve point); the coin log's append forest; the counting trie's root; a SHA-256 chain over the BIP30 entries emitted
so far, and their count.

**A chunk** (`methods/chunk`) reads S_in, the header times 0..H, its byte range, its hints (ECMH, P2PK y, tracked
outpoints) and a trie witness against S_in's trie root (checked to be exactly that root), processes its coins exactly
as the one-shot guest does, and commits **only** [SHA-256(S_in), SHA-256(S_out), SHA-256(header times), coins]. Chunk 0
alone may read the file header, and only from the initial state. The host computes every S_in natively: untrusted.

**The combine** (`methods/combine`) takes the chunk image id (an input, committed, as the zAPPs take the core id),
`env::verify`s each chunk journal under it, and requires: chunk 0 starts at the initial state; chunk i+1 starts at
chunk i's S_out digest; the last ends at the S_final it is given; every chunk used the times of the headers given here,
which link and end at S_final's base block; all announced coins read, the last group complete, chunk coin counts adding
up. It then finishes what no chunk can: pads and hashes the midstate (`hash_serialized_3`), closes the last group,
checks the BIP30 entry list against the chain and builds the tree (`sys` bulk), and builds the block forest. It commits
the one-shot guest's journal byte for byte, then the chunk id and N, so `ts-anchor join-receipt` rebuilds the same
anchor and link digest from it.

What this binding gives: no chunk can be dropped, repeated, reordered or overlapped (each breaks the digest chain), no
chunk can use other headers, and the only free inputs left are the ones the one-shot guest has too (the tracked and
REPEATABLE lists, §11.1). A dishonest host can only make the combine refuse.

### 12.3 Chunked ingest: correctness and negative controls (MEASURED)

- **Same anchor as the one-shot ingest.** The combine's journal equals the one-shot guest's journal byte for byte
  (`COMBINE_MATCH`) on: regtest 104 (3 chunks), regtest 110 (4), mainnet 200 (5), and the synthetic 1 M set (§12.4) in
  4, 8 and 16 chunks. The planner's own native expectation (`expected.journal`, the core's `Smt` for the BIP30 root)
  equals the one-shot journal in all of them.
- **Controls** (`experimental/trusted-state/chunk_controls.sh`, regtest 104, plan A = 3 chunks, plan B = 4, every
  chunk executed honestly; `CONTROLS_OK`):

| combine fed | result |
|---|---|
| A0 A1 A2 (honest) / B0..B3 (honest) | accepted / accepted |
| A0 + B1 B2 B3 (B1 starts at coin 43, A0 ends at 57: overlap) | refused: chunk 1 does not start where chunk 0 ends |
| A0 A2 (middle chunk dropped) | refused: chunk 1 does not start where chunk 0 ends |
| A0 A1 (last chunk dropped) | refused: the last chunk does not end at the final state |
| A1 A0 A2 (reordered) | refused: chunk 0 does not start from the initial state |
| A0 A1 A1 A2 (repeated) | refused: chunk 2 does not start where chunk 1 ends |
| A0 A1' A2, A1' = A1 with one header timestamp +1 (it executes fine on its own) | refused: chunk 1 used other header times |
| A0 A1 A2 with a final state of one satoshi more | refused: the last chunk does not end at the final state |

The chunk guest also requires its trie witness to be in counts mode (`wit[0] == 1`, the core's own condition): an
empty trie's root is zero in both modes, so without it chunk 0 could start a trie that commits no values or counts,
every later chunk continue it, and neither the combine nor the join would see the mode. The one-shot guest requires
the same, and that its witness opens the empty trie. `chunk_controls.sh` (given the controls build as its fourth
argument) feeds chunk 0 with its witness's mode word set to 0: refused by the real chunk image and by the controls
build without a cut, executed by the controls build with that check cut (21.11.2026).

### 12.4 Chunked ingest: measured at 1 M and 10 M coins

Synthetic snapshots (`ts-anchor synth`, Core's format, regtest headers to 110, P2WPKH coins, 2 per txid), with every
10th (1 M) or 100th (10 M) txid group a coinbase, so the BIP30 tree has 50,000 entries in both. NOT chain data.
`experimental/trusted-state/run_chunked.sh`: executor in dev mode (every guest executed, a fake receipt carrying its
claim so the combine can resolve it; nothing proved), default segment size 2^20, one process per chunk with its peak
RSS from `/usr/bin/time`, laptop (WSL2, 16 threads, 8.9 GB).

**1 M coins** (one-shot for comparison: the §6 guest with the `sys` BIP30 build, 19,507,304,084 user cycles,
20,534 segments, 440 s, 1.02 GB RSS; of it coins 6.47 G, trie 2.27 G, BIP30 tree 10.75 G = 215 k per entry):

| split | chunk user cycles (each) | chunks total | per coin (chunks) | peak RSS per chunk | combine | combine BIP30 tree | combine RSS / wall | **chunks + combine** | match |
|---|---|---|---|---|---|---|---|---|---|
| 4 × 250 k | 2,378,295,967 – 2,613,443,110 | 10,077,859,959 | 10,078 | 336 – 404 MB | 6,509,983,711 | 6,438,081,591 (128.8 k/entry) | 57 MB / 114 s | **16,587,843,670** | COMBINE_MATCH |
| 8 × 125 k | 1,188,944,753 – 1,345,932,848 | 10,366,936,536 | 10,367 | 193 – 237 MB | 6,510,011,615 | same | 57 MB / 114 s | **16,876,948,151** | COMBINE_MATCH |
| 16 × 62.5 k | 594,880,642 – 690,192,916 | 10,646,424,052 | 10,646 | 132 – 151 MB | 6,510,067,415 | same | 57 MB / 115 s | **17,156,491,467** | COMBINE_MATCH |

- A chunk costs more per coin than the one-shot's 8.74 k (coins + trie phases), +15% at 4 chunks to +22% at 16: every
  chunk after the first inserts into a non-empty trie through a witness (816,075 words for 125 k inserts into a
  trie of 125-250 k coins), and later chunks pay more (chunk 0 594.9 M, chunk 15 690.2 M at 16 × 62.5 k).
- Peak memory falls with the chunk size; the combine is small (57 MB).
- In the combine, the 16 journal checks cost 104,738 cycles (6.5 k per chunk); the resolution of the chunk receipts as
  assumptions (recursion, outside these counts) is NOT measured.
- The planner (`ts-anchor chunk-plan`, native, 8 threads): 28-31 s and 0.76-1.31 GB per 1 M plan.
- **Header times at mainnet height**, measured by re-running one 125 k chunk with its times array padded from 111 to
  935,001 entries (the chunk executes the same coins): +20,310,690 cycles (read + SHA-256 + table; 1,276,668,880 vs 1,256,358,190), i.e. about 20 M
  cycles per chunk at H = 935,000, independent of the chunk's coins. At H = 965,000 the same term, scaled linearly in
  the header count (as `scan-plan`'s `CHUNK_PER_HEADER` does), is ≈ 20.96 M (PROJECTED, not re-measured).

**10 M coins** (no one-shot comparison: a 10 M one-shot guest would need about 10× the 1 M run's 1 GB and its native
build about 10× its 3.3 GB, beyond this laptop; the combine is checked against the planner's own native journal, which
matched the one-shot journal in every case above). The planner kept the trie on disk (`--trie-dir`): 2,562 s
(42.7 min) on 8 threads for the 16-chunk plan, peak RSS 7.4 GB of which most is the memory-mapped trie arena (file-backed; anonymous memory
was about 1.8 GB when sampled during the run).

| split | chunk user cycles (each) | chunks total | per coin | peak RSS per chunk | wall per chunk | combine | combine BIP30 tree | **chunks + combine** | match |
|---|---|---|---|---|---|---|---|---|---|
| 16 × 625 k | 5,943,460,007 – 6,895,586,967 | 106,473,473,436 | 10,647 | 855 MB – 1.16 GB | 155 – 176 s | 6,510,757,943 (57 MB, 116 s) | 6,438,800,910 | **112,984,231,379** | COMBINE_MATCH |

| 8 × 1.25 M | 11,894,737,656 – 13,446,575,699 | 103,682,208,791 | 10,368 | 1.68 – 2.15 GB | 314 – 377 s | 6,510,702,143 (57 MB, 115 s) | 6,438,800,910 | **110,192,910,934** | COMBINE_MATCH |

The 8- and 16-chunk combines of the 10 M set commit the same journal (byte for byte, apart from the chunk count), and
both planners wrote the same expected journal. The 8-chunk plan took 2,610 s (43.5 min), peak RSS 7.3 GB (again mostly
the mapped trie).

Where a chunk's cycles go (16 × 625 k at 10 M, the guest's own counts per coin): the snapshot phase (parse,
`hash_serialized_3`, leaf, ECMH, coin log, trie key) is **flat at 7,216 – 7,232** (7,216.5 – 7,231.8: the `coins` count of
each chunk's `TSC` line / 625,000, MEASURED); the trie insert grows with the
trie it lands in: 2,267 (chunk 0, empty trie) → 2,704 (chunk 1, ~0.9 M) → 3,282 (chunk 8, ~5.3 M) → 3,455 (chunk 15,
~9.7 M), about 200-230 cycles per doubling; reading the trie witness grows likewise, 15 → 116 → 280 → 338 (13.1 M
witness words in chunk 15). The per-coin total over the whole 10 M build, 10,647, happens to equal the 1 M build's in
16 chunks (10,646): the larger trie costs more per insert, the larger chunks fewer re-reads of shared witness paths.

### 12.5 Mainnet projection (a PROJECTION from §12.1-12.4, not a measurement)

**N, the live coins at 965,000, is not known offline.** Core gives it as `gettxoutsetinfo`'s `txouts` (and
`dumptxoutset`'s `coins_written`, and the coin count in the snapshot's own header); chainparams'
`m_chain_tx_count` is a transaction count, not N. Everything is per 10^8 coins; K, the BIP30 entries (live coinbase
txids + tracked + REPEATABLE at 965,000), is not known either and is given per 10^5.

| part | measured basis | projection |
|---|---|---|
| chunks, per coin | snapshot phase 7.2 k (synthetic P2WPKH; early mainnet's uncompressed P2PK coins cost 8.7 k, §6); trie 3.46 k + witness 0.34 k at a 9.7 M trie, ~+215 and ~+60 per doubling | at 10^8 coins the build's mean trie is ~3.5×10^7 (log2 ≈ 25.1): trie ≈ 3.9 k, witness ≈ 0.45 k, so **≈ 11.5 k cycles per coin, ≈ 1.15×10^12 per 10^8 coins** (≈ 1.3×10^12 if every coin cost the P2PK rate) |
| chunks, fixed | header times: 20.3 M per chunk at H = 935,000 (measured by padding, §12.4), ≈ 20.96 M at 965,000 (scaled) | 160 chunks of 625 k per 10^8 coins: ≈ 3.35×10^9 |
| chunk size and memory | 625 k coins: 1.16 GB peak RSS at a 10 M trie, input ≤ 126 MB (13.1 M witness words) | the witness grows ~log: ≈ 1.2-1.4 GB per 625 k chunk at 10^8 (projection); 160 chunks per 10^8, all independent |
| combine, headers + forest | one-shot guest: ~2.2 k (headers) + ~2.5 k (block forest) per header | 965,001 headers ≈ 4.5×10^9 |
| combine, BIP30 tree | 129 k (combine) – 218 k (one-shot) per entry | **1.3-2.2×10^10 per 10^5 entries** |
| combine, per chunk | 6.5 k cycles to check a chunk journal (16 in 104,738) | negligible; resolving each chunk receipt as an assumption (recursion) NOT measured |
| native planner | 10 M: 42.7 min, 8 threads, disk trie, laptop | ≈ 7 h per 10^8 coins on this laptop if linear (the disk trie may scale worse; NOT measured) |

**In proving time**, at the header-chain zAPP's measured proving rate on one RTX PRO 6000 (≈ 0.66 M cycles/s,
LIBZYNC.md §4): ≈ 1.2×10^12 cycles per 10^8 coins ≈ **500 card-hours ≈ $1,250 per 10^8 coins** at $2.49/hr, spread
over 160 independent chunk proofs of ≈ 7.4×10^9 cycles (≈ 3 h each on one card); plus the combine, ≈ 2 h + 5.5-9.3 h
per 10^5 BIP30 entries on one card (the BIP30 tree is one receipt here; it can be split by key prefix into
independent subtree receipts, NOT built). Everything in this paragraph is a projection, and its rate is in user
cycles, which understates the chunks by about 9% (§13.4, "A bias in that rate").


### 12.6 Status, and what is not done

- Built and measured: the bulk BIP30 build (§12.1), the chunk and combine images and the planner (§12.2), the
  anchor equality and the controls (§12.3), 1 M and 10 M runs (§12.4). Executor only, dev mode; no receipt was proved.
- Not built: splitting the BIP30 tree by key prefix (it is one receipt in the combine); a streaming planner (it reads
  the whole snapshot into memory: 458 MB at 10 M, several GB at mainnet); the combine verifying chunk receipts
  recursively in a real prover run (its recursion cost; done in §14.1, 22.11.2026); real mainnet data above height 200.
- The free inputs are unchanged from the one-shot ingest (§11.1): the tracked and REPEATABLE lists.
- No core guest file changed (§7 still holds: `git diff origin/main...HEAD -- prover/methods libzync coinbase-smt
  accumulator` is empty).

## 13. Split BIP30 receipt, streaming planner, and the cost model (fourth PoC step, 21.11.2026)

Two of §12.6's "not built" items are now built (commits `92ee193`, `540cdbe`): the BIP30 tree split by key prefix into
independent piece receipts (§13.1), and a streaming planner, `ts-anchor scan-plan`, that reads a snapshot once and
estimates the cycles of the whole ingest (§13.2). The join receipt guest is §13.3. Everything is still executor only,
dev mode, on the laptop (WSL2, 16 threads); nothing was proved, and no core guest file changed
(`reproduce/LIBZYNC_IDS` is untouched; the core image stays `46666b6f`).

This section keeps two kinds of number apart. **MEASURED** means a guest's own cycle count, `/usr/bin/time`, or a
tool's output on this laptop, on the input named. **PROJECTED** means arithmetic on measured rates, applied to an input
I did not execute. Every number carries one of the two labels.

### 13.1 The split BIP30 receipt, measured at 10 M coins

**Design.** `ts-anchor bip30-plan` cuts the 2^d key prefixes of the BIP30 tree into P contiguous ranges of about equal
entry count and writes one input per piece; `ts-ingest-host bip30` executes the piece image, which builds the subtree
roots of its range and commits them with the entry and REPEATABLE chain digests at its start and end. The combine
(`--bip30`) verifies the pieces under the piece image id, requires the ranges to tile 0..2^d in order and the chains to
link to the entry list it closes itself, and folds the subtree roots into the root. `bip30_controls.sh` drives the
controls on regtest 104 (each refusal shown to come from the one check cut in the controls build).

**Run** (MEASURED, 21.11.2026). The 10 M synthetic set of §12.4 (P2WPKH, 2 coins per txid, every 100th txid group a
coinbase, 50,000 BIP30 entries, regtest headers to 110), `run_chunked.sh` with 16 chunks, the planner's trie on disk,
`BIP30_DEPTH=6 BIP30_PIECES=4`:

| step | user cycles | padded cycles | segments | peak RSS | wall |
|---|---|---|---|---|---|
| planner (`chunk-plan`, 8 threads, disk trie) | (native) | | | 7.89 GB (mostly the mapped trie, §12.4) | 2,743 s |
| 16 chunks of 625,000 coins | **106,473,473,436** (5,943,460,007 – 6,895,586,967 each) | 126,515,412,992 | 120,660 | 856 MB – 1.16 GB | 152 – 178 s each |
| BIP30 piece 0 (13,268 entries) | 1,731,713,187 | 1,803,026,432 | 1,720 | 49.7 MB | 30.7 s |
| BIP30 piece 1 (12,498) | 1,631,217,400 | 1,698,693,120 | 1,620 | 49.3 MB | 28.3 s |
| BIP30 piece 2 (12,504) | 1,631,999,115 | 1,698,693,120 | 1,620 | 49.3 MB | 28.2 s |
| BIP30 piece 3 (11,730) | 1,530,984,941 | 1,594,097,664 | 1,521 | 49.2 MB | 28.3 s |
| combine (`--bip30`, 16 chunks + 4 pieces) | **804,695** | 1,048,576 | 1 | 16 MB | 0.05 s |

The combine's journal equals the planner's expected journal byte for byte (`COMBINE_MATCH`), so the split tree has
the same root as the inline one. The 16 chunks reproduce §12.4's 16 × 625 k run to the cycle (the chunk image did not
change). Inside the combine (MEASURED, the guest's own counts): verify 102,797, read 42,465, chain + headers 260,941,
close + `hash_serialized_3` + entries 13,194, tree from the subtree roots 61,712, forest 309,264, commit 6,259.

What the split buys (COMPUTED from the table): the four pieces cost 6,525,914,643 user cycles in all, against the
inline tree's 6,438,800,910 in §12.4's combine, +1.4%; the largest single receipt in the BIP30 part falls from the
6.51 G inline combine to 1.73 G (piece 0), and the pieces are independent. Per entry the pieces cost **130,518.0 –
130,518.8**, i.e. the per-piece fixed part (read, chains and checks) is too small to show at this size.

### 13.2 The cost model of `scan-plan`, and its constants from this run

`ts-anchor scan-plan --snapshot <utxo.dat> --chunk-coins <C> [--height <H>]` streams a snapshot once (nothing but the
current chunk's counters in memory), cuts it into chunks of C coins, counts Core's script types, txid groups and BIP30
entries (coinbase groups) per chunk, and estimates each chunk's cycles with the rates in `mod rates`
(`experimental/trusted-state/anchor/src/main.rs`):

    chunk  = snapshot phase by script type + coins × (TRIE_A + TRIE_B × log2(mid)) + CHUNK_FIXED + CHUNK_PER_HEADER × (H + 1)
    BIP30  = entries × PIECE_PER_ENTRY
    combine = COMBINE_PER_HEADER × (H + 1)          (+ resolving the receipts, NOT measured)

`mid` is the trie size at the chunk's mid-point (coins before it + half its own). The snapshot-phase rates per script
type (one 100,000-coin chunk per type, `synth --script`, 21.11.2026, MEASURED, as recorded in `rates`): P2PKH 7,326.1,
P2SH 7,239.7, P2PK33 7,386.9, P2PK65 9,227.6, raw 22-byte (P2WPKH) 7,167.6, +21.1 per raw byte beyond 22.

**The three constants left open, filled from the 10 M run only** (COMPUTED from MEASURED guest counts):

- **TRIE_A, TRIE_B.** The per-coin trie term of chunk i is (`trie_read` + `trie`) / 625,000 from its `R0VM … TSC` line;
  x_i = log2(625,000 · i + 312,500). Measured: chunk 0 (9,605,504 + 1,417,037,629) / 625,000 = **2,282.6** at
  x = 18.2535; chunk 8 (174,787,150 + 2,051,088,924) / 625,000 = **3,561.4** at x = 22.3410; chunk 15 (211,552,179 +
  2,159,180,122) / 625,000 = **3,793.2** at x = 23.2077. Least squares over all 16 points: mean x = 21.841817, mean
  y = 3,408.4912, Sxx = 27.566870, Sxy = 8,258.7427, so **TRIE_B = Sxy / Sxx = 299.59** cycles per doubling and
  **TRIE_A = 3,408.4912 − 299.59 × 21.841817 = −3,135.1**. Residuals −50.8 (chunk 0) .. +27.7 .. −24.5 (chunk 15); the
  fit reproduces the 16 chunks' trie total, 34,084,912,032, exactly. The negative A only means the line is used
  above n ≈ 2^18; it is not a cost.
- **PIECE_PER_ENTRY** = (1,731,713,187 + 1,631,217,400 + 1,631,999,115 + 1,530,984,941) / 50,000 = 6,525,914,643 /
  50,000 = **130,518.3** user cycles per entry.

**Check against the same run** (COMPUTED: the model on `s10m.dat`, `--chunk-coins 625000 --height 110`, against the
MEASURED chunks). Model chunks 105,777,817,720 vs measured 106,473,473,436: **−0.65%** (−695,655,716), per chunk −0.07%
(chunk 0) to −0.92% (chunk 3). This is not an independent test, since the trie constants come from these chunks; the
residue is the other terms: the snapshot phase measured 7,216 – 7,232 per coin here against the 100 k-chunk rate
7,167.6, and the chunk read phase measured 6.51 – 6.56 M against the model's ≈1.06 M fixed part. The model's BIP30 term,
6.526e9, equals the measured pieces by construction. `scan-plan` itself (MEASURED) took 0.8 s and 7.1 MB peak RSS on the
458 MB file.

**A caution on the trie extrapolation.** The residuals bend (positive in the middle, negative at both ends): the local
slope over chunks 8..15 is (3,793.2 − 3,561.4) / (23.2077 − 22.3410) = **267.4** per doubling (COMPUTED), below the
fit's 299.6. Extrapolated 4.04 doublings further (to a 1.6e8 trie), the fit is therefore likely high, by up to
(299.6 − 267.4) × 4.04 ≈ 130 cycles per coin at the last chunk (COMPUTED); I keep the fit, which errs on the side of
cost.

### 13.3 The join receipt guest

The join image (`ingest/methods/join`, inputs from `ts-anchor join-inputs`) checks a real main@H receipt, the chunked
ingest's combine receipt and the anchored lineage's tip against each other, as §11.3 specifies; `join_controls.sh`
drives it on regtest H = 104 (the REAL `block_104.rcpt`, image `eb2cd424`, verified first; the anchored tip at 112),
with every control refused by the real image and accepted by the controls build with its one check cut. I do not
repeat its numbers here; this section is about the measurement above.

The join checks each core journal's `self_id` against the image it verified that receipt under (§11.3 step 2; cut
bit `CUT_SELF` = 32 in the controls build). Two controls drive it: main@H's journal with `self_id` changed and every
other field as committed (what the real core image commits when it is run with a rogue `self_id`; `ts-anchor
join-inputs --forge-main-self-id`, entering as a dev-mode claim under the real `main_image`), and the anchored tip's
journal with `self_id` changed (`--forge-tip-self-id`). Each is refused by the real join image and by the controls
build without a cut, and joined by the controls build with only `CUT_SELF` cut (21.11.2026), so the refusal comes from
that check. `ts-anchor`'s native `join`, `join-inputs` and `join-receipt` make the same check (`--image` /
`--main-image` and `--tip-image`). The five image ids plus the genesis anchor the join is given (six pins) must be pinned by the
verifier (§11.3, "What the verifier pins"); no pin file exists yet, and that is a release step.

What has no negative test: the native main-lineage `self_id` checks (`ts-anchor join` with a receipt, `join-inputs`,
`join-receipt`) only refuse a REAL receipt whose `self_id` is not its image, and making one needs a proved core
receipt run with a rogue `self_id`; `ts-anchor` refuses dev-mode receipts, so I have not built one. (`join` on a
state file with the wrong `--image`, and the tip checks, are tested in `join_controls.sh`.) The one-shot ingest
guest's mode and empty-root asserts (§12.3) have no committed control either: it has no controls build, and I checked
them by hand only (a mode-0 witness and a non-zero root are each refused by its own assert, 21.11.2026); the chunk
guest's identical mode check is driven both ways in `chunk_controls.sh`.

### 13.4 Projection at 1.6e8 coins (PROJECTED, not a measurement)

**Input.** No snapshot of that size is on this laptop and I did not run `dumptxoutset`, so I streamed a synthetic one
through a FIFO into `scan-plan` (nothing written to disk): `ts-anchor synth --coins 160000000 --height 965000 --base
<any> --coinbase-every 100 --stream --out <fifo>` and `ts-anchor scan-plan --snapshot <fifo> --chunk-coins 625000
--height 965000` (re-run 21.11.2026 for the new anchor; the first run was at 935,000). Same recipe as the 10 M set:
P2WPKH, 2 coins per txid, every 100th group a coinbase, so 80,000,000 txid groups and **800,000 BIP30 entries**;
7,687,107,851 bytes; header times to H = 965,000. MEASURED for the tools only: synth + scan-plan 25.4 s wall,
scan-plan peak RSS 3.3 MB, synth 7.3 MB.

| part | projected cycles (`scan-plan`) | projected segments | projected memory |
|---|---|---|---|
| 256 chunks of 625,000 coins | **1.888e12** (1,888,155,313,769; 11,800 per coin; first chunk 5.96e9, largest 7.645e9) | ≈ 2.14e6 (≈ 8,660 for the largest) | ≈ 1.41 GB peak RSS for the last chunk |
| BIP30: 16 pieces of ≤ 50,000 entries, depth 6 | **1.044e11** | ≈ 1.04e5 | not projected (measured 49 MB at ~12.5 k entries) |
| combine (965,001 headers + forest) | **4.536e9** + resolving 272 receipts (recursion: MEASURED per resolve in §14.1, ≈ 77 s on one card in all) | ≈ 4.3e3 | not projected (measured 16 MB at 16 + 4 receipts) |
| **total** | **1.997e12** | **≈ 2.25e6** | |

**What the move from 935,000 to 965,000 costs** (COMPUTED from the two `scan-plan` runs): only the header terms
change. The header-times term per chunk grows by 30,000 × 21.72 ≈ 0.65 M (20.31 M → 20.96 M, +3.2%), which is
+1.67e8 over 256 chunks (1,887,988,483,881 → 1,888,155,313,769, +166,829,888). The combine grows by 30,000 × 4,700 =
1.41e8 (4.395e9 → 4.536e9). Together that is +3.08e8, +0.015% of the total (1.99680e12 → 1.99711e12); the card-hour
figures below do not move at the precision given.

How the two derived columns were computed:

- **Segments** = projected user cycles × the MEASURED padded/user ratio of the 10 M run (chunks 126,515,412,992 /
  106,473,473,436 = 1.1882; pieces 6,794,510,336 / 6,525,914,643 = 1.0412) / 2^20 (default segment size). The combine's
  1.0 ratio is assumed, not measured at this size.
- **Chunk RSS**: a least-squares line of the MEASURED peak RSS of chunks 1..15 against log2(mid) (chunk 0 has no trie
  witness): 61.6 MB per doubling, evaluated at the last chunk's mid (log2 159,687,500 = 27.25): ≈ 1,407,000 kB. The same
  fit on the trie witness puts the last chunk's witness at ≈ 24.0 M words (measured 13.1 M at chunk 15 of the 10 M run).

**In proving time** (PROJECTED, at the header-chain zAPP's measured proving rate on one RTX PRO 6000, ≈ 0.66 M cycles/s,
LIBZYNC.md §4, as §12.5 uses): 1.997e12 user cycles ≈ 3.0e6 card-seconds ≈ 840 card-hours, ≈ $2,090 at $2.49/hr, of which
the 256 chunks and 16 pieces are independent receipts (the largest chunk ≈ 3.2 h on one card).

**A bias in that rate.** The 0.66 M cycles/s is in USER cycles: 2,350,725,289 user cycles proved in 59.4 min, MEASURED
(LIBZYNC.md §4). That run's padded cycles were 2,437 segments × 2^20 = 2,555,379,712, a padded/user ratio of **1.087**
(COMPUTED), so the card proved ≈ 0.717 M padded cycles/s. The prover's work follows padded cycles, and the chunks pad
more: 1.188 (MEASURED at 10 M, above), the pieces 1.041. At the padded rate, with each part at its own measured ratio
(combine at 1.0, assumed): 1.888e12 × 1.1882 + 1.044e11 × 1.0412 + 4.536e9 ≈ 2.357e12 padded cycles / 0.717e6 ≈
3.29e6 card-seconds ≈ **913 card-hours, ≈ $2,273** at $2.49/hr (PROJECTED; 919 card-hours if every part padded at
1.188), and the largest chunk ≈ 3.5 h on one card. This assumes proving time is proportional to padded cycles across
guests, which I have not measured. §12.5's proving-time paragraph has the same bias: its ≈ 500 card-hours per 10^8
coins becomes ≈ 550 (≈ $1,370) at the chunks' ratio, and its ≈ 3 h per chunk ≈ 3.4 h (PROJECTED).

**What this projection does not know.** Mainnet's N and K (the live coins and the BIP30 entries at 965,000) are not
known until the snapshot is taken (§9); 1.6e8 and 800,000 are this synthetic set's, not mainnet's. Mainnet's script mix is not P2WPKH
only: at the measured per-type rates an all-P2PK65 set would cost 2,060 more per coin than raw-22 in the snapshot phase
(9,227.6 − 7,167.6, MEASURED rates), so a real snapshot has to go through `scan-plan` itself. The trie term is
extrapolated 4 doublings beyond the largest measured trie (§13.2's caution). The recursion cost of resolving 272
receipts in the combine was not measured here; §14.1 measures it (≈ 77 s on one card, +0.002%). Against §12.5's earlier projection of ≈ 11.5 k cycles per coin, this model
gives 11.8 k per coin at 1.6e8.

## 14. Real proofs, the recursion cost, the host's disk stores and a verifier prototype (fifth PoC step, 22.11.2026)

Branch `feat/trusted-state` merged with origin/main (#779, #783, #785, #787; no conflict, merge `7827a2c0`); the three
control scripts pass on the merged tree (`CHUNK_CONTROLS` `CONTROLS_OK`, `BIP30_CONTROLS_OK`, `JOIN_CONTROLS_OK`,
22.11.2026). Still no core guest file changed: `git diff origin/main...HEAD` touches `experimental/trusted-state/`, this
file and the four host files only (`prover/host/Cargo.toml`, `src/lz.rs`, `src/main.rs`, `src/trusted_state.rs`), so the
core images are unchanged (mainnet `46666b6f`). Labels as in §13: **MEASURED** (a run named here) or **PROJECTED**
(arithmetic on measured rates).

### 14.1 Real proofs of the chunked ingest and the join: the recursion cost

Until now every receipt of the ingest was a dev-mode claim, so the cost of the combine and the join **resolving** their
receipts (one recursion proof per receipt they `env::verify`) was not measured (§12.4, §12.6, §13.4). Here the whole
chain was proved for real, on CPU, end to end.

**How.** `ts-ingest-host` gained `TS_PROVE=1` (dev mode off): the guest is executed, every segment proved
(`prove_segment`), merged and lifted (`prepare_lifts`), joined pairwise to one receipt, then each assumption resolved
(`resolve`, one call per receipt the guest verified) and the result verified against the image id; a `TSP` line times
each stage. `run_chunked.sh` passes `TS_PROVE=1` through. The join takes the anchored tip as a REAL receipt
(`TS_TIP_RCPT`, verified under the tip image, its journal required to be `tip.journal`). The anchored tip is block 105
proved for real by the core host on the anchor at 104: the host built on hazync-coord with the CI regtest guest
embedded prebuilt (`HAZYNC_GUEST_ELF`, sha `1c3d671a…`, image `eb2cd424`, the image of the real main@104 receipt) and
`--features trusted-state`, `HAZYNC_SEG_PO2=20`, `HAZYNC_CHUNKS=1`, `HAZYNC_TS_SMT_SEED`. So main and tip are the same
image here, as the design allows (§11.3 pins both).

**Box.** hazync-coord (AMD EPYC 9645 VM, 16 vCPU, 62 GB), CPU prover (risc0 3.1.0, `prove` feature, no CUDA), every run
`nice -n 10`, segment size 2^20 for the ingest images. **Shared**: the Ghost spine's two `fold-range` provers ran the
whole time (load average 40-48 on 16 vCPU), so every wall time below is a contended-CPU time, comparable within this
section only. No GPU, nothing paid.

**The chain proved** (MEASURED 22.11.2026, `/root/ts14w/run_r104.sh`; all real receipts, each verified):

| receipt | user cycles | padded (segments) | segment proofs | merge + lifts | joins | resolves | wall | peak RSS |
|---|---|---|---|---|---|---|---|---|
| anchored block 105 (core image `eb2cd424`, base step on the anchor at 104): chunk + trie + aggregate | 1,275,832 + 65,421 + 2,559,376 | 1,572,864 (2) + 262,144 (1) + 3,145,728 (3) | (inside the host) | | | (the aggregate resolves its chunk and trie receipts) | **1,664.7 s** | 10.4 GB |
| ingest chunk 0 / 1 / 2 of Core's `utxo_104.dat` (57 / 56 / 56 coins) | 594,265 / 618,336 / 613,276 | 2^20 (1) each | 294.3 / 204.5 / 214.3 s | 27.0 / 18.7 / 21.5 s | — | — | 321.5 / 223.3 / 235.8 s | 9.6 GB |
| BIP30 piece 0 / 1 / 2 (depth 4; 100 entries + 1 REPEATABLE in all) | 5,083,393 / 4,682,036 / 4,544,799 | 6 / 5 / 5 segments | 1,121.1 / 941.7 / 949.9 s (186.9-190.0 s each) | 121.4 / 93.0 / 94.6 s (18.6-20.2 s each) | 5 / 4 / 4: 102.0 / 71.8 / 75.9 s (18.0-20.4 s each) | — | 1,344.7 / 1,106.7 / 1,120.5 s | 9.6 GB |
| combine (`--bip30`, 3 chunks + 3 pieces) | 620,991 | 2^20 (1) | 189.8 s | 17.6 s | — | **6: 109.6 s (18.27 s each, 17.69-18.77)** | 317.1 s | 9.6 GB |
| join (main@104 REAL GPU receipt + the combine receipt + the anchored block 105) | 119,707 | 2^18 (1) | 46.1 s | 19.0 s | — | **3: 55.3 s (18.44 s each, 17.71-19.42)** | 120.4 s | 2.4 GB |

Results: the combine's journal is the planner's expected journal byte for byte (`COMBINE_MATCH`); the join guest
**JOINED** (join image `a7f37798…` of this build): H = 104, anchor link `8fce9ea6…` (the ANCHOR_ID of §5.1), T = 105.
So the anchor at 104, built from Core's snapshot, is shown in one chain of real proofs to be the deterministic function of
the snapshot (ingest), equal in every JOIN field to the real from-genesis receipt at 104, and the base of a real proof of
block 105. Every image id here is a hazync-coord build (paths), not a pinnable id (§11.3).

**What a resolve costs.** All 9 resolves took 17.7-19.4 s, mean 18.3 s, in a combine of 6 assumptions and a join of 3:
flat in the number of assumptions and in the size of the receipt resolved (a 1-segment chunk, a 6-segment piece, the
core aggregate), and the same as one lift (17.6-27.0 s) or one join (18.0-20.4 s) on this box. That is what risc0's
recursion predicts (a resolve is one fixed-size recursion program over two succinct receipts), now MEASURED. A resolve is
about 1/10 of proving one 2^20 segment here (18.3 s against 186.9-294.3 s).

**Folded into §13.4** (PROJECTED, 1.6e8 coins, 256 chunks + 16 pieces):

| term | basis | projection |
|---|---|---|
| resolves in the combine | 272 receipts, one resolve each | 272 |
| resolves in the join | main@H, ingest, tip | 3 |
| on this CPU box (contended) | 18.3 s per resolve (MEASURED above) | 275 × 18.3 s ≈ **84 min**, serial (a resolve consumes the previous one's output) |
| on one GPU | 0.28 s per resolve, MEASURED before on the core aggregate's chunk resolves (docs/history/TEN_MINUTE_BLOCK.md §8.15 (b); 16 resolves in 4.2-4.7 s, docs/history/SEGMENT_DISTRIBUTION.md) | 275 × 0.28 s ≈ **77 s ≈ 0.02 card-hours ≈ $0.05** at $2.49/hr |
| making each chunk and piece receipt succinct (its own lifts and joins) | already in §13.4's rate: the header-chain zAPP's 59.4 min is for ONE succinct receipt of 2,437 segments, lifts and joins included | no new term |

So the recursion §13.4 left out adds about 77 s on one card to the ≈ 913 card-hours (≈ $2,273) projected there:
**+0.002%, below the precision of that figure**, plus about 77 s of serial latency at the end of the combine (the
chunks and pieces stay independent). On CPU alone the same 275 resolves would take about 1.4 hours on this box. The
projection's other caveats (§13.4: N and K unknown, the script mix, the trie extrapolation, the padded-cycle bias) are
unchanged; the recursion cost is no longer among them.

<!-- 14.1b: an 8-chunk combine (11 resolves), filled in below -->

### 14.2 The host at mainnet scale: the anchor written into the host's disk stores (§8.7)

**What changed.** `ts-anchor build --coinlog-dir <d> --trie-dir <d>` writes the anchor's coin log and counting trie
straight into the stores the host already runs on (`HAZYNC_COINLOG_DIR`: the coin-log nodes and `index.bin`;
`HAZYNC_TRIE_DIR`: the trie arena), in the host's own formats (`libzync::host::DiskNodes` / `DiskIndex`,
`libzync::trie_store::TrieStore::open_dir`). `<out>.coinlog` is then the host's disk-mode state file (`LZDISK02`: height
H, leaf count, roots; what `cl_save` writes and `cl_load` checks) and `<out>.trie` its disk marker. With a disk trie (and
no `--ingest`) the inserts are applied in batches of 2^20 (the trie root is a function of the set; `chunk-plan` already
applies per chunk and matches the one-shot root), and only coinbase and tracked txid groups are kept (the REPEATABLE
check runs in the pass). The host needed **no change** to read these stores: `cl_init_at_start` / `trie_init_at_start`
already open a disk-mode anchor and check it against the anchor's `utxo_roots` / `utxo_leaves` and `trie_root`, height
and block. A run advances the stores in place, so an anchor's stores serve one run (copy them to keep the anchor).

**`HAZYNC_TS_SMT_SEED` with a resume** (it refused one before). A resume rebuilds the BIP30 tree at the last receipt k;
the main lineage replays it from block files 1..k, which a Trusted State run does not have. Now the run writes
`state_<h>.smt` (the tree's entries) beside each receipt, before the coin log and trie saves (the last two kept, as
theirs), and a resume loads `state_<k>.smt` and requires its root to be **block_k.rcpt's `smt_root`**, which the guest
proved. That is sound for the same reason the main lineage's replay is: the file is trusted for nothing, the proved
root decides, and block k+1's guest checks its BIP30 witness against that same root. All of it is behind the
`trusted-state` feature; a default host refuses `HAZYNC_TS_SMT_SEED` (checked: "needs a host built with --features
trusted-state") and its resume and save paths are the same code as before (`ts_run` is the constant `false`).

**Controls** (`experimental/trusted-state/disk_controls.sh`, regtest anchored at 104, blocks 105..112, dev mode,
laptop host with the prebuilt regtest guest `eb2cd424` and `--features trusted-state`; `DISK_CONTROLS_OK`, MEASURED
22.11.2026):

| check | result |
|---|---|
| the disk-store anchor's `.json` and `.smt` vs the in-memory build | byte-identical, ANCHOR_ID `8fce9ea6…` both |
| the anchored lineage 105..112 from the disk stores | `state_112.json` byte-identical to the in-memory run's |
| a run stopped after 108, resumed with `HAZYNC_RESUME=1` (in memory; and on disk) | "resuming after block 108", `state_112.json` = the unbroken run's, both |
| resume with one value of `state_108.smt` changed | refused: "its N entries do not hash to block_108.rcpt's smt_root" |
| resume with `state_108.smt` missing | refused (the file is required) |

**At 10^6 and 3×10^6 coins** (MEASURED, laptop, WSL2; `ts-anchor synth --coinbase-every 10` on the regtest headers to
110, so 50,000 / 150,000 BIP30 entries; peak RSS sampled from `/proc/<pid>/status` every 0.3 s and split into
anonymous and file-backed memory; the disk rows are the final code, which flushes the disk trie after every batch):

| | 10^6 in memory | 10^6 disk stores | 3×10^6 disk stores |
|---|---|---|---|
| `ts-anchor build`: ANCHOR_ID | `25fdd21b…7af1` | `25fdd21b…7af1` (`.json` and `.smt` byte-identical to in memory) | `8ab4378e…2a52` |
| `ts-anchor build`: peak RSS, anonymous | **3,164,500 kB** | **684,732 kB** | **1,212,540 kB** |
| `ts-anchor build`: peak RSS, file-backed (the mapped trie arena; the kernel can drop it) | 3,908 kB | 1,850,012 kB | 2,643,556 kB |
| `ts-anchor build`: wall (native, one thread) | 29.9 s | 37.0 s | 110.6 s |
| on disk | `.coinlog` 32.0 MB, `.trie` 139.8 MB | coin-log store 115 MB, trie arena 2.0 GB (`du`) | 424 MB, 2.7 GB |
| host, block 111 as a base step on the anchor, dev mode: peak RSS, anonymous | 1,347,020 kB | 1,117,012 kB | not run |
| host: `state_111.json` | — | byte-identical to the in-memory run's | — |

Block 111 is `coinbase_only_block.py` of the harness's block 111 (every transaction dropped, the coinbase paying the
subsidy alone, its witness commitment, merkle root and nonce redone): a valid block that spends nothing, so it can be
proved on a synthetic anchor whose coins are not the chain's.

Before the per-batch flush the disk build kept every trie slot an insert replaced in memory until its one final flush:
2,043,792 kB anonymous and a 6.2 GB arena at 3×10^6 (MEASURED); with it, 1,212,540 kB and 2.7 GB, the same ANCHOR_ID.

What this shows and what it does not:
- The in-memory build's 3.2 GB per 10^6 coins is gone: the anchor's coin log and trie live in the host's stores, and
  the host opens them as it opens its own (same `state_111`, 0.23 GB less anonymous memory at 10^6; the host's
  remaining ~1.1 GB is the executor, not the stores).
- **The disk build's anonymous memory still grows**: 684,732 → 1,212,540 kB from 10^6 to 3×10^6, about 0.26 GB per
  10^6 coins (MEASURED slope). A trie-only run (coin log in memory) grows the same way (853,368 → 1,332,884 kB), so it
  is not the coin-log store; the trie arena's in-memory free list (the replaced slots of a batch, reused by the next)
  and the batch itself are the candidates, NOT profiled. PROJECTED, if that slope held: ~42 GB anonymous at 1.6e8
  coins, against ~506 GB for the in-memory build (3.16 GB × 160). So the build fits a large box but is not yet
  constant-memory; finding the growing structure is open.
- PROJECTED (not run): the trie arena grew 2.0 → 2.7 GB from 10^6 to 3×10^6 (0.35 GB per 10^6 at the margin, 0.9 on
  average), so ~56-144 GB at 1.6e8 coins; the coin-log store 115 → 424 MB (~155 MB per 10^6), ~25 GB.

### 14.3 Verifier support, as a prototype (§8.1)

**Where it lives.** The checks §8.1 needs are in `swiftsync-verify` (the core verifier library, its CLI, C ABI and
`.wasm`) and are reached by every zAPP through `zapps/corecheck::verify_core`. Both are shared with main, and a change to
either moves the core verifier's and the zAPPs' WASM shas. They do not need to change: `swiftsync_verify::verify` already
takes `anchor: Option<ChainState>` and, with an anchor, runs every check (receipt, image, `self_id`, profile, time, block
forest shape, lineage, `anchor_id == link_digest(A)`, work, checkpoint) and returns DEMO; `anchor_id()` and the `Image`
fields are public. So the prototype is a separate crate that wraps them unmodified:
`experimental/trusted-state/verify` (`ts-verify`, own workspace). The zAPP side would be the same wrapper over
`verify_core`'s report, in each zAPP's own module; not built.

**What it adds.** A pinned table `ANCHORS: (network, H, block, hash_serialized_3, anchor_id, source)` and a verdict
`TS_VERIFY_ANCHORED` (exit 3) between `TS_VERIFY_OK` (0, a genesis-anchored proof, exactly swiftsync-verify's OK) and
`TS_VERIFY_DEMO` (2). ANCHORED requires: swiftsync-verify's checks all pass with the given A; a pin exists for
(network, A.height); A's tip is the pinned block; `link_digest(A)` is the pinned anchor_id (not PENDING); the proof's
tip is above H; and the work and checkpoint an OK would need. It prints §4's label with H, the block, the
`hash_serialized_3`, T and the image. Entries:

| network | H | block | hash_serialized_3 | anchor_id | status |
|---|---|---|---|---|---|
| mainnet | 965,000 | `00000000000000000001595977e6000ce56129f5c9b4073e31ccc30b90b97da9` | `4a8d794337118c0c615b574f817c7306c687584a537184b8d233df42bf477ec2` | **PENDING** (`None`) | the snapshot is not taken (§9); to be filled only from `ts-anchor build` on Core's dumptxoutset at 965,000, reproduced by a second party, and not before the ingest receipt of that snapshot exists (§11.1). A unit test fails if it is filled |
| regtest | 104 | `6abf594e…2fb8` | `97aa0565…9015` | `8fce9ea6…f71e` | TEST ENTRY (the zAPP harness chain), not a network pin |

The regtest image (`eb2cd424`, genesis anchor `99509fc5…cf63`, profile 0x79) is carried by the prototype as
`zapps/corecheck` carries it under its `regtest-test` feature; a unit test recomputes the genesis anchor.

**Controls** (`experimental/trusted-state/verify_controls.sh`, MEASURED 22.11.2026, `VERIFY_CONTROLS_OK`), on REAL
receipts: main@104 (`block_104.rcpt`, GPU, 16.11) and the anchored lineage's block 105, proved for real on CPU on
hazync-coord as a base step on the anchor at 104 (§14.1; core image `eb2cd424`, the CI regtest guest embedded
prebuilt, host `--features trusted-state`):

| input | verdict |
|---|---|
| main@104, no anchor | `TS_VERIFY_OK` (exit 0) |
| anchored block 105 + anchor_104.json | **`TS_VERIFY_ANCHORED`** (exit 3) |
| anchored block 105, no anchor | FAIL: `anchor_id is genesis` |
| anchored block 105 + the anchor with `max_time` + 1 | FAIL: `anchor_id is the GIVEN anchor` (the link digest moves) |
| anchored block 105 with one seal byte flipped | FAIL: `receipt verifies against image eb2cd424…` |
| main@104 + anchor_104.json | FAIL (a genesis proof is not on A) |
| the real build with the controls hook set | FAIL: the hook needs the controls build |
| controls build (`--features controls`), pin unchanged | ANCHORED |
| controls build, the regtest pin replaced by PENDING | DEMO: "the regtest anchor at 104 is PENDING" |
| controls build, another anchor_id pinned | DEMO: "the anchor's link digest 8fce9ea6… is not the pinned anchor_id" |

Not built (still §8.1): the joined statement after the join receipt; history facts relabelled "since H" (the
prototype checks no fact; its label says history starts at H); coin and tx facts below H on the 0x02 block-forest
leaves; the zAPP-side wrappers. The pin is trusted: it is only as good as the reproducible build behind it.
