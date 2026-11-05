# libzync: roadmap to the freeze and beyond (11-03)

Everything built, discussed or promised so far, in build order. Status words: **done**, **building**, **next**,
**experiment** (built behind a flag, measured, then decided).

⚠ **Testing policy until phase H:** unit tests and compiles only. No long suites (rig, handover, signet from genesis)
until everything is built and reviewed; then one full test pass on the final image.

## A. Chain identity

| # | item | status |
|---|---|---|
| A1 | guest pins one checkpoint (`checkpoint.rs`: mainnet 970,000, signet 325,000); verifiers give DEMO below its height | done (c8 compiles, 9d9adb54 before B1) |
| A2 | `--expect` / `--mempool` / `--mempool-url` in both verifiers, refuse mistyped flags, unit tests | done (`3785f56`, `8e47047`) |
| A3 | host verifier `--bitcoin-cli` | done |

Minimum work stays a verifier check: every proof below the tip is low-work by construction, so the guest cannot
refuse it.

## B. zyncAPP coin facts (ZYNCAPPS.md, COINLOG.md §6)

| # | item | status |
|---|---|---|
| B1 | `libzync::facts`: coin leaf, unspent / spent-by, block in chain + state at a height, tx in block, tx parsing, `check_core_statement` | done |
| B2 | host writes `history.bin`; `zync-path coin` / `find` produce coin facts | done |
| B3 | coin journey: creating tx in a proven block, the coin leaf from its output, the spending tx (`zync-path journey`, `--journey`) | done |
| B4 | `swiftsync-verify --coin / --journey`, wasm `verify_facts`, C ABI `swiftsync_verify_facts` | done |
| B5 | zyncAPP `coin` guest (prover/zyncapps): unspent/spent at the core tip, core receipt verified in the zkVM; image 26e09637 | done |

## C. Production host

| # | item | status |
|---|---|---|
| C1 | disk-backed coin-log store (`HAZYNC_COINLOG_DIR`, DiskNodes) + C1b on-disk position index (DiskIndex) | done (history.bin replay is still in memory) |
| C2 | `zync-serve` path service (GET /tip, /coin, /find); payment hook not yet | done |

## D. Transaction log (txproofs/README.md)

| # | item | status |
|---|---|---|
| D1 | Phase A: speculative chunks (zero hash/time) accepted from CSV on when flags need no hash; host `HAZYNC_SPECULATIVE` | done (image c9tx df86c392) |
| D2 | aggregate record selection (`HAZYNC_TXLOG_ORDER`, each record once) | done |
| D3 | Phase B (context-free records): NOT needed — Phase A waste measured 7.5% of pre-proved inputs over 8 blocks (txproofs §8) | dropped |
| D4 | `fetch_template_rpc.py` (template -> speculative fixture); waste measured 7.5% (txproofs §8) | done |
| D5 | zyncAPP `zeroconf` guest: speculative chunk record + every input unspent at the tip | building |
| D6 | Ghost proof-ready blocks: the pool runs D4 on its own template (Ghost-side integration; Ghost infra is separate) | after D4 measures |

## E. Proof of absence

| # | item | status |
|---|---|---|
| E1 | keyed trees for absence (outpoint / script), built as guest features otree / stree and MEASURED on 10 blocks: +55.6% one tree, +110.7% both, almost all in the aggregate (absence/README.md); key-prefix sharding measured: no saving in work, only splits serial execution — not pursued; history-keeping script tree measured +49.7% alone / +100.1% with the outpoint tree, BUT undercounted (within-block spent coins not inserted; README correction 11-04) | measured |
| E1b | corrected history tree (every created coin, within-block spends inserted marked) and the counting trie (one two-level trie: address then coin, history kept, per-address unspent count and value; `ttree.rs`), images c10h2 / c10t. MEASURED (9 modern blocks): corrected history tree +60.5%; **counting trie +51.3%, every fact in one tree** (counts ~free; 1,024 coins per address: +44% / +82% on 962,000 / 750,000) | measured |
| E1c | counting trie TRIMMED (one pass over the witness, hashes in place, cheaper chunk keys, one-compression address leaf): **+28.0% over 9 modern blocks (was +51.3%), 962,000 8.40x vs main** (c10t5: full 256-bit address binding, coin key fix); at its hashing floor (absence/README.md) | **CHOSEN 11-04 (user): the path forward** |
| E2 | integrate the counting trie (c10t5 design) into the core: trie root in ChainState bound proof to proof (not from the witness); the trie updated in its OWN receipt in parallel with the chunks and verified inside the core proof (split measured: serial aggregate 192 M -> 99 M, whole-block cycles +2.4%; pinned in the core image like the chunk program); host trie store; absence / history / balance verifiers (cli, wasm, C ABI); zync-serve endpoints | **done 11-05.** Steps 1-2 (7ab30d4, c10e5 ce96f2ec): trie as guest mode 17, receipt verified in the aggregate, root in the chain state; negatives refused; product +19.9% over the pre-hunt no-tree base on 9 modern blocks, 962,000 9.09x vs main. Step 3 (7e00760, 1c63de5; c10e7 026ebf10): host trie store (`libzync::trie_store`, snapshots `state_<h>.trie` beside the coin log, all-or-nothing apply, undo), proved and committed per block in prove-range; test genesis..200 + resume at 120 (same root) + tampered snapshots refused + 4 negatives refused (`trie_store/RESULT_c10e7.txt`). Step 4 (82a31d6, 806d9e1): coin / balance / history facts (`libzync::trie_fact`) in swiftsync-verify `--trie`, the C ABI and WASM (`"tries"`), `zync-path trie`, `zync-serve /trie/*`; checked on the real 0..200 run (block 9's coinbase spent at 170 by tx 1 input 0; 5 coins ever paid to that script, 28 BTC left). Cleanup: the wire's hint-bit field, the hints-file branch of every verifier and the v1 bundle removed; ChainState fields renamed `trie_root` / `live_set` / `live_hash` (JSON aliases read the old names) (bad3c7d; c10e8 e8f9ac1b: the store test again, identical, `trie_store/RESULT_c10e8.txt`). Review round 1 (a4a7c40): trie facts carry their claimed answer (`expect`), the C ABI requires one; product GPU runs checkpoint the trie. ✅ 11-05 mainnet scale: the store is an ARENA of 64-byte records, in memory or memory-mapped on disk (`TrieStore::open_dir`, host `HAZYNC_TRIE_DIR`), copy-on-write (an apply is all or nothing; slots freed only after a flush, so a crash leaves the last flushed trie); disk = memory block for block (roots, witnesses, snapshots), crash test passes. Sizing: 4.04 nodes / 258 bytes per coin on a synthetic chain with little address reuse; real mainnet bytes per coin to be measured by a prep run (estimate hundreds of GB: needs NVMe on the prep / tip box) |
| E2b | instant IBD: the chain state's ECMH total tracks the LIVE coin set (`utxohash`), so a new node verifies a downloaded UTXO set (~10 GB) against the proof. Measured +4.56% padded over 9 modern blocks (+1.2-1.8% on recent blocks). **CHOSEN 11-04 (user)**; part of E2 | chosen |
| E2c | consolidation (after E2a batches 2-3, before E2 integration and the reviews): ONE product configuration (coin log + counting trie as a parallel receipt + live-coin hash; signet / regtest / txlog kept); delete the dead experiments from the guest (otree / stree / htree / itree, design B + hints file, msm / field-bench / liftx-accel / tx-cycles, trie inside-vs-split switch); split `libzync` into the core library (coin log, trie update, facts, wire) and tooling / bench code kept out of anything that ships; break host main.rs into modules (prover, verifier, tools; benches out of the main binary). Cost table + unit tests green after each step | **done 11-05** (10161bf, 361e4f5, b26ca4f, host split): product image 16d9409b; every step's aggregate journal identical to batch 2's apart from the image-id fields. (The hint-bit field, hint hash and hints-file branch went in E2 step 4.) ⚠ pre-existing host test failure smt_bridge::a_fully_spent_coinbase_can_be_duplicated_with_no_special_case (BIP30 coinbase duplication in the bridge SMT) — review rounds |
| E2a | cycle hunt across all core units (chunk, aggregate, trie guest): profiles in `~/libzync-gpu/e1/prof`, three read-only agents, each candidate measured on the cost table before it lands. Batch 1 (whole blocks -0.8 to -12.5%), batch 2 (aggregate -28.5%, whole -6.4%), batch 3 (trie guest -11.3%) | done 11-04 (more trie work possible later) |

## F. main branch

| # | item | status |
|---|---|---|
| F1 | port the `is_base` forgery fix and the BIP68 fix to `main` | draft PR #668 (merge + deploy: user) |

## G. Docs

| # | item | status |
|---|---|---|
| G1 | spec text for each item as it lands (SWIFTSYNC_PROOFS, COINLOG, ZYNCAPPS, txproofs) | with each item |
| G2 | **final repo cleanup (user, 11-05): a concise, readable repo, minimal docs and tools, no fluff, precise info.** Every experiment-era doc, script, bench and result moves to `experimental/` (or is deleted): the retired design-B docs (SWIFTSYNC_PROOFS, CORE_HOOK, UTREEXO_FORMAT), this ROADMAP, the reference scripts in `scripts/libzync` (keep only what a test or a reproducible id needs), the zyncapps bench guests (hashbench, triebench), checked-in test outputs. Live spec content those docs still carry (the coin leaf, UTREEXO_FORMAT §2; the genesis anchor rule) is folded into a few short specs first, and code comments re-pointed. Then decide the historical names (`swiftsync-verify*`). Hazync's own docs (docs/*.md, ~25: coordinator, fleet, proving, soundness, ...): libzync IS Hazync (the new library), so each doc that is relevant and current stays or is updated, stale ones go, and the many overlapping ones are condensed / combined into fewer | after the freeze (H7), before the paper |
| G3 | **the paper, written WITH the user** (not drafted alone): what libzync proves, how, measured costs, soundness argument, limits | after G2 |

## H. Path to the core freeze (the user's gates, in order)

1. ~~E1 tree decision~~ — **done 11-04: the counting trie (E1c) is the path forward.**
2. **E2a** cycle-hunt batches 2-3, then **E2c** consolidation (one product configuration, dead experiments deleted, library split), then **E2** integration (trie as a parallel receipt inside the core proof, live-coin hash) — moves the image.
3. Several cold review rounds on everything since `00b7ac2` (feedback: reviews before any freeze).
4. One full test pass on the final image: rig vs Core, Core's own test vectors (I1), handover, mainnet 1..200, signet
   from genesis, edge blocks, cost table.
5. **GPU proving benchmarks** on the final image (user's trigger).
6. **Freeze: ask the user.**
7. After freeze: container ids built twice, LIBZYNC_IDS + verifier IMAGES tables (cli/ffi/wasm), vectors regenerated
   from GPU run A, proposal page refresh. A full mainnet proof is the user's trigger. ⚠ review round 3 (11-05): until
   then the built-in IMAGES (9f469930 / 067822b9, from c1101a3 10-31: ChainState v1, profile 1) verify NOTHING current,
   so OK is reachable only via `--image` / `image_hex`, and the table's `checkpoint_height: 970_000` describes an image
   with no checkpoint: replace both with the frozen product images (cli, ffi `IDS`, wasm `image_id`). Also decide a
   proof-age policy (today a genuine year-old proof is OK; facts are "as of its height").
8. **G2 repo cleanup** (concise repo, experiment-era material to `experimental/`), then **G3 the paper with the user**.

### H-B. Backfill design: anchored blocks + one join tree (agreed with the user 11-05)

The original Hazync's shape (independent block proofs folded into one chain proof; its prove_range / fold_range) was
right; the separate anchoring stage goes. Today libzync is strictly LINEAR (block N's aggregate verifies N-1's
receipt): fine at the tip, a serial bottleneck for a whole-chain backfill.

1. Prep (host, native): the exact chain state before every block, S0 (genesis) .. S_tip: coin log, trie, BIP30 tree,
   block forest, work, times. Not trusted, only starting points.
2. Leaves: any block, any order, any number of GPUs: block h proved as a base step on S(h-1) ("from the state whose
   digest is anchor_id, block h is valid and gives S(h)"). Already supported (the anchor path).
3. Joins: a NEW guest mode (~50 lines): env::verify left and right (same image), require sha256d(left journal) ==
   right.anchor_id, same kind / profile / lineage; commit the right state with the left's anchor_id. The full-journal
   digest covers every state field at once. A binary tree over N blocks is N-1 joins (= the N-1 links), ~20 levels for
   ~975 k blocks, every level parallel. A wrong host state only fails a join (digest), never forges.
4. Tip: unchanged (linear step, or prove on S_tip and join onto the root: the same statement).
5. Published: ONE receipt (root, genesis -> tip; optionally Groth16-wrapped) + its chain state. WASM verifies that one
   receipt (pinned image, start == the genesis anchor else DEMO, tip vs a trusted source); "block X is valid" = a
   block-forest path from X to the root's block_roots; coin / trie facts against the same state.
6. Across an image change: the handover step, not a join.

Steps: (a) join mode + host `prove-anchored` / `join` commands (c10e11, before the freeze); (b) TEST: blocks 1..200 as
200 independent anchored proofs, joined: the root journal must be byte-identical to the linear run's block_200;
(c) negatives: gap, overlap / duplicate, swapped order, tampered anchor, other image, non-chain journal — all refused;
(d) in the GPU benchmark step: a REAL (non-dev) join of joins (the dev-mode test shows byte-equivalence, not the
recursion: env::verify / assumption resolution and "succinct, joinable again" need real receipts), and MEASURE join
cycles / cost (the old fold was $0.0043, another guest), arity (2 / 4 / 8) and leaf size; (e) after the freeze: the coordinator's work queue (prep ahead, leaves in any order, joins as children
land, resume points).

## I. Gaps beyond the core (11-04)

What SwiftSync + Utreexo + libzync still do not solve, and how each is closed. None changes the core image unless a
test finds a bug.

| # | gap | how | status |
|---|---|---|---|
| I1 | trust in our guest | Core's own test vectors (`script_tests`, `tx_valid`, `tx_invalid`) through the guest; fuzzing; extend the rig | **harness built 11-05 (c909841)**: guest mode 10, `scripts/libzync/core_vectors.py` (1,420 base cases = Core 28's counts, 22,666 with flag variants), `host core-vectors`; **c10e10, base set, libsecp path: 1,420 / 1,420 agree with Core, 0 differ** (exact ScriptError for script_tests; 68 by a guest abort where Core throws and catches inside EvalScript and fails the script: the guest has no C++ unwinding, both reject; `core_vectors/RESULT_c10e10_base.txt`). **No unwinding, by decision (user, 11-05):** every catch the guest can reach rejects the block (EvalScript, signet), pinned in guest build.rs `REVIEWED_CATCHES` (the build walks the include graph and fails on any change), the host reports an abort as `BLOCK REFUSED`, CONSENSUS_RULES.md states it. ⏳ variants (22,666); the BATCHED signature path (mode 10 runs libsecp, no hints): an adversarial run (host hints R = lift_x(r) for every call: no invalid case may pass) and an honest run (real R: every valid case passes), next image; fuzzing; extend the rig |
| I2 | trust in our guest + verifiers | external audit and a bug bounty before anyone relies on proofs for money | after freeze |
| I3 | trust in the zkVM | pin one risc0 version and track its audits; optional: prove periodic checkpoints on a second zkVM (e.g. SP1), so one prover bug cannot forge alone (~2x cost, checkpoints only) | later; user's call |
| I4 | data availability | the blocks are the data: publish coin log / tree snapshots (verifiable against proven roots, any untrusted mirror works) + a "rebuild from my own node" tool | after freeze |
| I5 | proving cost | the 10x target (E1); one proof per block shared by every user; zyncAPP revenue pays (payment is the rate limiter); speculative pre-proving (D1) moves work off the critical path | ongoing |
| I6 | not consensus | Ghost pool commits the proven roots in its coinbase (no fork; miner attestation on chain); wallet / exchange adoption via the verifiers; a soft fork is a community process for much later | I6a Ghost coinbase: after freeze (Ghost-side, user's call) |
| I7 | tip latency | speculative chunks (D1, 7.5% waste measured), zeroconf zyncAPP (D5), more cards per block; trim the tree's +14-29 s execution | ongoing |
| I8 | privacy of queries | whole-snapshot download and local queries; batched / padded queries; private information retrieval later | later |

## J. SwiftSync: what Hazync can offer Ruben (11-05)

An offer, not a dependency: SwiftSync stays as it is, Hazync only removes its worst failure mode (a bad hints file is
found only at the END of a sync). Proofs of concept from design B are kept at tag `archive/swiftsync-design-b`
(806d9e1, the last commit before E2 removed them). The hand-over is a one-page note the user edits and sends.

| # | idea | what exists | status |
|---|---|---|---|
| J1 | **SwiftSync zAPP: a proven hints file for ANY height H.** The core already proves every block and commits the coin log (and the block forest, each block's coin state); a zAPP walks the log as it stood at H, emits one bit per output (unspent at H), commits (H, block hash at H, sha256(hints)). Anyone can serve the file; a node checks the receipt against a Hazync tip proof BEFORE syncing, then still validates every block itself (the proof buys liveness, not trust). A new H is a new zAPP run, not a new checkpoint scheme: the "moving H" answer | design only | candidate, after the freeze. First: (a) the hints index EVERY output in block order, the log holds only coins that outlived their block and are not OP_RETURN (all 0 bits): check a coin leaf (or the tx log, D) gives each survivor's position in its block and each block's output count; (b) MEASURE the cost (every log leaf up to H; parallel by range + join; once per H) |
| J2 | **proven hints file inside the proof (design B).** Ruben's check on a proof: `ss_set == 0`, `height == H`, `ss_hints == sha256(file)` | built + GPU-proved: signet 2501..2600 verified on GPU; verifiers (cli / ffi / wasm) took the hints file | retired from the core in E2 (tag); J1 is its zAPP form |
| J3 | **the coin log IS a hints file for every H**: leaves in birth order, a spend flips its leaf in place, so a leaf's state at H is that output's hint bit; one structure serves hints for any H as it moves with the tip | the coin log (COINLOG.md), live in the core | idea for Ruben; the basis of J1 |
| J4 | **BIP30 done exactly**: only the transactions that can ever repeat are tracked (pre-BIP34 coinbases indicating a later height, and every tx whose inputs are all such coins), with live counts: a repeat is refused while it has unspent outputs, allowed once it has none, as Core. Measured on mainnet to 974,111: 173 such transactions for indicated heights under 100M, 22 outputs unspent, 1 after block 300,000; the grandfathered 91,842 / 91,880 overwrites. Any hints-based sync must get these right | `coinbase-smt/src/bip30.rs`, 29188b5, rig 83/84 agree with Core 28.0 | data + method to share |
| J5 | **hints-format hygiene**: canonical encoding rules, a Python reference, malformed-file vectors (flipped bit, non-canonical size, trailing byte, bad version), a hint-hash definition both sides compute the same way | at the tag: `scripts/libzync/ss_hints_ref.py`, `swiftsync-verify/vectors/bad_*.hints`, `libzync/src/ss_hints.rs` | ready to share as is |
| J6 | **SwiftSync + moving H + Utreexo**: SwiftSync to reach H fast, a Utreexo accumulator to carry the tip with no UTXO set; J3 bridges the two. The coherent end state for SwiftSync (NOT for Hazync, whose design stays the coin log + trie) | design notes: `experimental/libzync-bench/HANDOVER_DESIGN.md` (option B) | idea for Ruben |
| J7 | the note: one page, J1-J6, what each is, what was measured, where the code is | not written | after the freeze, user edits and sends |

## K. zAPP ideas for the relaunch (11-05)

Hazync relaunches on libzync (the new core library) with the zAPP ecosystem around it. Cheap, visible zAPPs first: each
is a marketing win that shows the core working.

| # | zAPP | what it proves | cost | status |
|---|---|---|---|---|
| K1 | **header chain** | the most-work chain of valid proof-of-work headers from genesis: double SHA-256 under target, nBits and the 2,016-block retarget, median-time-past, chainwork. SPV-level (no transactions, coins or scripts); the core already runs these checks inside every block proof. Kept at the tip at a fixed few seconds a block (one header ~3 k cycles, far below the smallest segment) | ESTIMATED, not measured: ~2-3 k cycles a header (3 compressions: 283 cycles each on words, 1,105 per 65-byte `sha2` hash, measured; the rest small) => ~2-3 B cycles for ~975 k headers => ~40-60 min on one RTX PRO 6000 (1.2 s per M cycles, measured) ~$2, or ~1.5-2.2 h on an A40 ~$1; minutes on 16 cards. About the cost of 5-7 full modern blocks | **BUILT 11-05** (zyncapps guest `headers`, `zyncapp headers-exec`): Core's header rules (prev link, CheckProofOfWork, CalculateNextWorkRequired, time-too-old, BIP34/66/65 versions, GetBlockProof work; target and work computed once per nBits). Executed on REAL mainnet headers from the node: 0..2,100 (through the first retarget) end hash and chainwork **identical to the node's**; tampered nonce / time / bits / prev refused (high-hash, time-too-old, bad-diffbits, link). **MEASURED: 2,427 cycles per header** (genesis..2,100). Whole chain ~975 k headers = ~2.4 B cycles; at the MEASURED core proving rate (1.2 s per M cycles, PRO 6000) ~47 min ~ $2, or A40 ~1.8 h ~ $1 (proving time an estimate from cycles until proved). ⏳ the recent range across two real retargets (969,696, 971,712); a proved run; joins of runs (state digest in, state out) for the tip. Launch line: "the whole Bitcoin header chain, proved, for about $2" |
| K2 | SwiftSync hints for any H | see J1 | see J1 | candidate |

## L. Business model for zAPPs (11-05; reasoning, not measured demand)

The code is open source, so it is not the moat. Open SDK (anyone builds zAPPs) + a curated registry + our paid proving
network. A marketplace grows out of the registry and the proving service; it is not built first, and builders are
not gated (that cannot be enforced, and slows the ecosystem).

| # | lever | why it pays | revenue |
|---|---|---|---|
| L1 | **registry of reviewed zAPP image ids** | a zAPP proof means nothing unless the verifier trusts its image id; whoever publishes the reviewed, reproducibly built ids is the gatekeeper wallets / exchanges rely on (the app-store position) | review + listing fees for third-party zAPPs |
| L2 | **proving network + live state** | every zAPP needs the proven tip and its state (coin log, trie, block forest) kept current per block, plus GPUs: cheap to write a zAPP, expensive to run this | pay per proof, subscriptions for tip-following; a take rate on third-party zAPPs proved on it. Payment is the rate limiter (as the coordinator) |
| L3 | **first-party zAPPs for buyers with budgets** | proof of reserves (exchanges, custodians: coins unspent at the tip, summing to X) — the clearest paying market; Bitcoin state for bridges / L2s (BitVM-style bridges, sidechains, rollups) — likely the most lucrative; instant IBD / UTXO-set checks (node and wallet vendors); wallet light clients (balance, history, absence: volume, low price) | per proof / per query / contracts |
| L4 | marketing zAPPs | header chain (K1), SwiftSync hints (J1): cheap, visible, not revenue | none directly |

Order: freeze the core -> ship 2-3 first-party zAPPs (header chain for the launch, then proof of reserves and coin /
balance facts) -> open the SDK with the reviewed-id registry -> talk to 2-3 exchanges and one bridge team BEFORE
building more (their answers set the pricing). Cheapest real test of demand: ask them.

## Image ids built (11-03, unit tests + compiles only)

| image | id | note |
|---|---|---|
| c8m (checkpoint) | 9d9adb54 | superseded |
| c9tx (coin log + txlog) | df86c392 | |
| c9m (coin log) | 0d8cac45 | ⚠ built while a zyncAPP build held part of target/riscv-guest: rebuild from clean before the test pass |
| zyncAPP coin | 26e09637 | |
| zyncAPP zeroconf | a61caf76 | |
| c10b / c10o / c10s / c10os (E1 measurement) | e7514dc8 / 629005ae / 3c948172 / 95df867e | tree state from the witness; cost only |
| c10h / c10oh (E1 history script tree) | b704daf3 / cba70157 | ⚠ undercounted (within-block coins); superseded by c10h2 |
| c10h2 / c10t (corrected history tree / counting trie) | ab3f1ac1 / a87a541e | tree state from the witness; cost only |
| c10t2 / c10t3 / c10t4 (trie trims 1-3) | 00ec71b3 / 73158901 / 58bab460 | tree state from the witness; cost only |
| c10t5 (trie, final measured) | c7c1463e | tree state from the witness; cost only |
