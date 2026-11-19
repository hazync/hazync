# Trusted State: zAPPs before the backfill reaches the tip (proof of concept, planned for v0.28)

Status: **EXPERIMENTAL proof of concept**, branch `feat/trusted-state-poc` (19.11.2026), laptop CPU only, not reviewed.
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
| 3. guest takes the anchored state as base and validates further blocks | **yes, with no guest change**: `is_base = 1` (the existing base step) | regtest anchored at 104, blocks 105..112 (incl. the BIP30 repeat at 105) in the executor (dev mode) |
| 3b. image id | **unchanged** (no guest source touched; laptop mainnet image rebuilt from clean: same id and ELF) | §7 |
| 4. cost | ingest in a guest: §6; per block anchored vs main: §5.4 | executor, laptop |
| 5. zAPP on the anchored state | balance statement: **identical answer** to the main lineage's; script history: **only from H** | §5.5 |

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

The build is deterministic (the same inputs gave the same ANCHOR_ID on every run) and refuses: a `hash_serialized_3`
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
> anchor's side inputs (headers 0..H, BIP30-tracked coins). Not yet joined to a proof from genesis.

Every anchored proof is today a DEMO verdict (its `anchor_id` is not the genesis anchor); the label needs a pinned
table `(network, H, block, hash_serialized_3, anchor_id)` in the verifiers, as `genesis_anchor_hex` is now, and a
new verdict between OK and DEMO. Not built here.

**The join rule** (`ts-anchor join`): given the anchor A (its JSON), a receipt R_H of the main lineage at H that
verifies under a published image with the network's genesis `anchor_id`, and an anchored tip proof R_T:

1. `R_T.anchor_id == link_digest(A)` (the anchored proof rests on exactly A);
2. equal in A and R_H: `kind`, `version`, `tip_hash`, `height`, `utxo_value`, `cum_work`, `prev_nbits`, `prev_time`,
   `epoch_start`, `recent_times`, `max_time`, `live_set`, `live_hash`, `profile`, `block_leaves` (every field a later
   block's verdict reads, except the BIP30 tree, §3.3);
3. NOT compared, each lineage's own: `utxo_roots` / `utxo_leaves` (the logs differ: all coins ever vs live coins),
   `trie_root` (history vs none), `smt_root` (§3.3), `block_roots` (UNKNOWN states below H), `self_id`, `lineage`,
   `anchor_id`.

Why that suffices: the anchored log's leaves are exactly A's coins (A is a deterministic function of the snapshot; a
verifier either recomputes it or takes the ingest proof of §6), and `live_hash` equality makes them, as a multiset of
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
mainnet 200 it is the value ASSUME_ZAPPS.md §5 recorded from Core 31.1.0. Every build ran twice with the same ANCHOR_ID.

### 5.2 Join check (question 2): anchored state vs a REAL from-genesis proof at the same H

`ts-anchor join --main <receipt> --image <id>`: the receipt is verified first (risc0 3.1.0 verifier, dev mode compiled
out), then decoded.

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
UNKNOWN there, so no coin path opens), spends before H, coins created and spent before H, and a script's complete
history. Transaction-in-block facts for blocks ≤ H do open (the leaf carries the real block hash), but they rest on
the header chain bound to the pinned block hash, i.e. on the assumption.

<!-- INGEST -->
