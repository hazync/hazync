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
| 3. guest takes the anchored state as base and validates further blocks | **yes, with no guest change**: `is_base = 1` (the existing base step); one host hook for the BIP30 tree | regtest anchored at 104 (blocks 105..112, incl. the BIP30 repeat at 105) and at 101 (102..112); mainnet anchored at 199 (block 200); executor (dev mode); every resulting state = the real receipts' |
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
   ≤ H (`undo_from` = H+1 for an anchored chain); and history facts relabelled "since H" (§5.5).
2. **The side inputs are not covered by Core's hash.** `hash_serialized_3` covers outpoint, height, coinbase flag and
   output, not the tracked bit, the REPEATABLE slots or the creation MTP. The headers are bound by hashing; the
   tracked and REPEATABLE lists are checked only by a later join (a wrong one moves `live_hash`: §5.3 controls). On
   mainnet no tracked coinbase can repeat below 1,983,702, so before the join a wrong list changes leaf bytes, never a
   verdict; still, an anchor should not be pinned until the lists are reproduced independently (e.g.
   `assume_inputs.py`'s rule over the blocks by a second party), or until the join. Mainnet has 22 live tracked
   outputs at 974,111 (ASSUME_ZAPPS.md §3.2); the REPEATABLE count at 935,000 is not known here.
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
   `HAZYNC_TS_SMT_SEED` refuses a resume (the PoC replays nothing): both small host changes, not made.

## 9. Mainnet: the anchor height and an estimate path

**Anchor height.** Core's newest mainnet `m_assumeutxo_data` found offline: **height 935,000**, block
`0000000000000000000147034958af1652b2b91bba607beacc5e72a56f0fb5ee`, `hash_serialized_3`
`e4b90ef9eae834f56c4b64d2d50143cee10ad87994c614d7d04125e2a6025050`, `m_chain_tx_count` 1,305,397,408 (a local
checkout of Core master at `2586ed68`, 30.03.2026, version 31.99; earlier entries 840,000, 880,000, 910,000). The
v28.0 tree the guest is built from pins only 840,000. A newer Core release may pin a later height; not checked.

**What is ESTIMATED below** chains measured unit costs; nothing here was run at mainnet scale.

| step | unit cost (MEASURED) | mainnet quantity | estimate |
|---|---|---|---|
| native anchor build (`ts-anchor`, one thread, in memory) | 96 s and 3.2 GB per 10^6 coins (§6) | N live coins at 935,000: **not known here** (Core's `gettxoutsetinfo` gives it) | ~96 s × N/10^6 one thread before the disk stores; the ECMH part is parallel (ASSUME_ZAPPS.md §5: 1.9 µs/coin on 16 threads) |
| ingest receipt (optional, §8.6) | 8.8 k cycles/coin, 1.46 M cycles per BIP30 entry, 4.7 k cycles per header (§6) | N coins, K entries, 935,001 headers | 8.8×10^11 cycles per 10^8 coins + 1.46×10^11 per 10^5 entries + 4.4×10^9; at the header-chain zAPP's proven rate (2.35×10^9 cycles in 59.4 min on one RTX PRO 6000, LIBZYNC.md §4, ≈ 0.66 M cycles/s) ≈ 370 card-hours per 10^8 coins + 61 per 10^5 entries |
| anchored lineage 935,001 → tip | per block = main's (§5.4); block 962,000 proved in 537 s on one RTX PRO 6000 (LIBZYNC.md §4) | 40,954 blocks to 975,954 (hazync-coord's tip, 15.11.2026) | ≈ 6,100 card-hours, ≈ $15 k at $2.49/hr, from that one block's time; real blocks vary |
| for scale: the main lineage from genesis | — | 975 k blocks | $278-295 k on PRO 6000 (WHOLE_CHAIN_COST.md, itself ESTIMATED, ±50%) |

So the anchored lineage is roughly 5% of a from-genesis proof's cost (ESTIMATED), paid a second time for the blocks
above H (§8.4), and it brings balance-type zAPPs to the tip once those ~41 k blocks are proved, under the §4 label,
until the main lineage reaches 935,000 and the join discharges the label.

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
2. requires the ingest's (H, tip, value, live set) = main@H's, main@H genesis-anchored, the same profile;
3. requires J_T.`anchor_id` == `link_digest(A)`. Because A's header fields were taken from main@H, this one equality
   also proves the anchor's time context and work were main@H's: rule 2 of §4 for those fields comes for free;
4. commits the joined statement: both image ids, the genesis anchor, H and its block, T and its tip, chainwork and
   `max_time` of T, and the digests of J_H and J_T, so a split answer (§11.2) checks its main@H facts against J_H's
   roots and its anchored facts against J_T's.

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

<!-- REPLAY -->
