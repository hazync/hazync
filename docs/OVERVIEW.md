# Hazync overview

What Hazync proves, its goals and where each stands, how the original system works, where libzync fits, and a
glossary. Merged from the September 2026 `EXPLAINER`, `GOALS`, `DESIGN_OVERVIEW` and `GLOSSARY` (now in
[`history/`](history/README.md)).

Numbers come from committed evidence (`prover/evidence/`, `docs/history/`) unless marked INFERRED or ESTIMATE.
Sections 1–3 describe the **original system**: the canonical CORE guest `37987b85`, the coordinator and the
fleet. Its code is still in the repo. Section 4 covers libzync, the core that is replacing it.

Related: [`SPEC.md`](SPEC.md) (formats, invariants) · [`SECURITY.md`](SECURITY.md) (soundness, threat model,
reviews, fuzzing) · [`OPERATIONS.md`](OPERATIONS.md) (proving, fleet, tip runs, settings) ·
[`COORDINATOR.md`](COORDINATOR.md) (routes, running one, sponsorship) ·
[`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md) (CORE/GHOST builds, releases) ·
[`../CONTRIBUTING.md`](../CONTRIBUTING.md) (join the board) · [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md).

---

## 1. What Hazync is and proves

A Bitcoin full node re-checks every block from 2009 before it trusts the current coin set. Hazync proves that work
once, inside a RISC Zero zkVM, and publishes a receipt anyone can check in milliseconds, with no node, no peers and
no chain data.

The guest runs **Bitcoin Core's own consensus code**, compiled to RISC-V, rather than a rewrite of the rules. A
verified chain proof attests: every block from genesis to the tip is valid under Core consensus, the UTXO set equals
the committed root, and the work is as committed. That covers scripts of every type, real ECDSA and Schnorr through
`libsecp256k1`, no inflation, proof-of-work and difficulty, merkle and witness commitments, weight, sigops and the
locktime/BIP rules, under Core's exact flags.

**Compiled from Core:** `interpreter.cpp`, `SignatureHash`, `CheckTransaction`, `ComputeMerkleRoot`, the
transaction/weight/sigop machinery, the retarget (`pow.cpp`'s `CalculateNextWorkRequired`, driven through the real
`CBlockIndex`) and `libsecp256k1`, with zero consensus-logic changes. Four patches, all in `patches/`:

| patch | target | what |
|---|---|---|
| `0001` | Core | ILP32 `Serialize` overload |
| `0002` | Core | SHA-256 compression through the zkVM accelerator, byte-identical output |
| `0012` | libsecp | coprocessor field backend, at the interface libsecp already parameterises (since v0.21.0) |
| `0013` | libsecp | `lift_x` witness hint, checked by libsecp's own arithmetic before use (since v0.21.0) |

wNAF, GLV, the ECDSA and Schnorr logic and every check above the field stay libsecp's. The guest is therefore
*maximal-Core*, not pure Core; [`SPEC.md`](SPEC.md) §12 lists every non-Core piece of the circuit.

**Not compiled from Core:**
- the subsidy halving schedule and the script-flag activation heights, each differentially tested against Core (the
  flag schedule is proven a sound superset of `GetBlockScriptFlags`). The compiled retarget is also cross-checked
  against the on-chain `nBits` at all 476 mainnet retargets;
- the Utreexo UTXO accumulator (`prover/methods/guest/src/utreexo.rs`) and the BIP30 coinbase SMT: our own code. The
  accumulator was differentially fuzzed ~900k executions against a reference model (`audit-fuzz/`).

**Reviews.** Two external reviews ran in August 2026 (rounds 10 and 11,
[`history/SECURITY_AUDIT_LOG.md`](history/SECURITY_AUDIT_LOG.md)). They were AI-assisted code reviews, not a
commissioned professional audit, which has still not happened. Neither found a way to make the guest accept an
invalid chain. Both found real defects, now fixed or tracked, and both named the C++ shim layer and the accumulator as
the residual risk; one found panic paths in the reference crate, now fixed. The most serious finding of that period
was internal: audit #3 found that the guest would **reject** a valid chain at block 91841, because blocks 91842 and
91880 duplicate still-unspent coinbases and the new non-membership check had no exception for them. Fixed in v0.15.0;
the real blocks are now fixtures. Audit record: [`../SECURITY.md`](../SECURITY.md).

**Not a goal:**
- *Beating a full node on trust.* The claim is a smaller trust assumption than the status quo: Core already skips
  signature checks for most of the chain via `assumevalid`, on a developer-chosen hash. Hazync replaces that anchor
  with a proven one.
- *Proving anything but mainnet.* The guest compiles `CChainParams::Main()`; another network needs another guest and
  image id. (libzync adds default signet, section 4.)

Prior art: [ZeroSync](https://zerosync.org) (Robin Linus and collaborators). The zkVM is
[RISC Zero](https://risczero.com): `vendor/risc0-zkvm` is their 3.0.5 crate with two local changes
(segment-distributed assembly, a balanced join tree), `vendor/risc0-circuit-rv32im-sys` their 4.0.3 crate with one
prover-side fix for #119. Attribution: [`../THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).

## 2. Goals

Six goals. A task that cannot be traced to one is not on the critical path.

| goal | statement | status |
|---|---|---|
| G1 | a receipt per block, permanently retained | achieved, at risk |
| G2 | one receipt, genesis to tip | mechanism proven; scale is the problem |
| G3 | verify on small compute | met via WASM (2026-08-01) |
| G4 | rapid IBD from a proof | mechanism met on mainnet (2026-08-05); not at scale |
| G5 | validate hazed blocks against a proof | met (2026-08-05) |
| G6 | prove blocks faster than they arrive | not met; sized |

Dependencies: G2 needs G6 to converge; G5 needs G1's proofs; G4 needs a proof at a useful height, not the tip; G3 is
independent. Only G2 depends on G6.

### G1. A proof per block

A sceptic must be able to check one block alone. CI does: `prover/testdata/snark/neg500.snark` is a Groth16-wrapped
mid-chain range (regenerated 2026-09-07), and `.github/workflows/adversarial.yml` asserts `hazync-verify` exits `2`
on it (valid, not genesis-anchored), the verdict every per-block receipt above block 1 gets. The retained count is
reported by `coordinator/check-retention.py` (CI proves it can still fail: `coordinator/test-check-retention.sh`).
The risk is folding deleting leaves; since v0.13.0 folding is a separate task over already-submitted receipts
(`hazync fold`). Retention is a storage decision: ~215 GB at tip.
**Done when** per-block receipts are retained and served for every proven block under any work scheme (#37).

### G2. Genesis to tip

`fold_1000` is a genesis-anchored proof of blocks 1..1000 in **3,441 bytes**, verified in CI and on ARM64. A
genesis-anchored spine is live at `/api/spine`. Current figures belong only on the [live board](https://hazync.org/)
(`/api/state` as JSON); every re-baseline resets the board, because receipts from a superseded guest do not verify
under the new one. Every canonical id is in [`../reproduce/LINEAGE.tsv`](../reproduce/LINEAGE.tsv).

**Cost by inputs, not blocks.** Early blocks are nearly empty (block 20,000 holds 19,023 UTXOs in total).

| measure | value | label |
|---|---|---|
| block 741,000 (670 inputs), stock guest | 3,275 s GPU, 16 chunks | measured |
| block 962,000, stock guest, L40S, po2 22 | 17,340 s = 4.8 h; **2.41 card-s/input** | measured 2026-08-24 |
| block 966,108 (8,562 prevouts), CORE, 8 × L40S, po2 21 | **~0.77 card-s/input** (5,075 chunk + ~1,500 aggregate card-s) | INFERRED (`history/BENCH_8xL40S_2026-09-08.md`) |
| historical board rate | 2,220 blocks/hr, on near-empty blocks only | measured |
| Bitcoin's history | ~1.8–3.0 billion inputs | ESTIMATE |
| whole chain on CORE | **~44–73 L40S card-years** (stock: 138–229) | INFERRED from one near-tip block |

⚠ Read the order of magnitude, not the digits. Extrapolating 2,220 blocks/hr (~17 GPU-days) was wrong by orders of
magnitude; the ~17 GPU-years that replaced it was ~10× too low (a near-tip block took 289 min of card time). Money
cost has not been re-derived for CORE. At CORE's rate a 1% cycle saving is worth roughly half a card-year of backfill
(⚠ the quoted 1.8–2.3% for `ECMULT_WINDOW_SIZE` is the 15 → 19 move; window 21 measured −1.245%).

**The bridge is no longer the bound.** On 2026-07-30 it ran ~291 blocks/hr, projecting ~881 days to tip. A/B over
100 blocks from h=182,310, bundles byte-identical in all arms:

| arm | per block | blocks/hr |
|---|---|---|
| original | 12.455 s | 289 |
| cached internal nodes (`Forest` stored only leaves: 94.1% of time) | 0.697 s | 5,163 (17.9×) |
| + leaf-position index (then 71% of what remained) | 0.067 s | 53,492 (185.1×) |

The rate tracks inputs per block, so the tip-era figure needs its own measurement.
**Done when** one genesis-anchored receipt covers block 1 to a current tip and verifies against the canonical
`METHOD_ID`.

### G3. Verify on small compute

WASM verifier, deployed at <https://hazync.org/verify/>:

| | |
|---|---|
| peak linear memory | **1.9 MiB** (27 pages at instantiate, 30 after 20 verifies) |
| verify time | 21–46 ms (x86-64, `[1..8]` proof of 1,841 bytes) |
| download | **~295 KB** gzipped, 1,064,517 bytes raw |
| imports | none (no WASI, no JS callbacks, no threads); wasm MVP |

WASM linear memory is architecture-independent, so the x86 figure transfers. Demonstrated 2026-08-01: the deployed
module is byte-identical to the signed release, reports the canonical id, and returned `verified` for the live spine
(71 ms) and `not_anchored` for a mid-chain segment (46 ms) (`prover/evidence/wasm_verifier_live.txt`). No run on a
physical phone or Pi is recorded; native `hazync-verify-aarch64` RSS was only run under qemu (#41 closed as
superseded). A small device should fetch the SNARK wrap (**1,841 bytes** for `[1..8]`, not the ~200–300 B in older
docs). CUDA Groth16 crashes (#20), so wrapping is CPU-only.

### G4. Rapid IBD

A ghostd node (bitcoin-ghost/ghost#543, set binding #101) loads a UTXO set it never validated on the authority of a
genesis-anchored proof and validates on from the proven height. Verified against the real mainnet header chain:
exact coin count, a coin inside the range present and the first past it absent, background validation disabled, a
restart with the proof returns to the adopted state, a restart without it is refused. The dump comes from the
bridge (`host dump-snapshot`) and is checked by rebuilding the accumulator roots; one flipped byte is refused.
`getblockchaininfo.hazync.utxodumpmatched` reports the verdict. No developer-chosen hash is in the trust chain,
unlike Core's `assumeutxo`.

Demonstrated at height 8 (`-hazyncproof=fold_8.snark -hazyncutxo=dump_h8.bin`): the mechanism, not the saving.
Separately, *validate-with-elision* (still downloads every block, skips scripts) was shown at height 1000: UTXO set
byte-identical, 8/8 adversarial inputs refused. Blocks 1..1000 hold ~1,020 transactions, so a meaningful timing
needs a proof at 200k–400k (GPU time, not code). Adoption needs only the base block in the **headers** chain.
**Done when** a node syncs to a checkpoint height from a proof in seconds with a chainstate byte-identical to
conventional validation.

### G5. Hazed blocks

A node holding only stripped blocks binds them to an existing proof. Run on a real archive (ghostd v1.10.16,
`-hazemode=hazed`, 1,255 stripped blocks): txids read from the hazed archive, all 1,000 merkle roots recompute, all
headers link and meet their targets, the hazed tip equals the proven tip. Identity comes from the stored blocks,
validity from the proof; no GPU needed. ghostd does this natively with the `hazyncverifychain` RPC
(bitcoin-ghost/ghost#627) at any proven height, optional `from_height` (the result says whether the whole chain or a
suffix was established). It checks against the archive's headers, not the block index, and refuses before the walk
if the proof commits to a different tip.

### G6. Tip-following

Without it G2 cannot converge. CORE guest, near-tip blocks:

| framing | fleet | label |
|---|---|---|
| one block inside 600 s | ~13 L40S | INFERRED from 8 × L40S at 15m13s on 966,108 |
| keep pace at a bounded lag | ~11 L40S | INFERRED, ~6,570 card-s per block on 966,108 |
| one block, rented RTX 4090s | 26 cards **8.09 min**; 27 cards **9.07 min** (544.0 s) | MEASURED on 966,256 (`history/MILESTONE_966256_RUN4_2026-09-10.md`) |

⚠ Name the framing, card and po2 with any fleet size. 24 GB cards run po2 21 (peak 22,478 MiB on a 4090); po2 22
peaked at 40.6 GB on the stock guest. The stock guest's ~29 (and `history/FLEET_SIZING.md`'s ~32) are superseded.
The bridge produces a witness per 0.067 s at h~182,000; re-check at tip-era input counts.
**Done when** sustained throughput exceeds one block per 10 minutes over a period that includes large blocks.

### Backfill and tip are two workloads

| | backfill (G2) | tip-following (G6) |
|---|---|---|
| work | ~44–73 L40S card-years (INFERRED) | ~11 L40S continuously (INFERRED) |
| sized by | budget and calendar | the block interval |
| per-block latency | irrelevant | the only thing that matters, eventually |

They add: ~11 cards is break-even and burns down no backlog. +50 L40S backfills in ~0.9–1.5 years, +100 in
~0.4–0.7 (INFERRED). Tip latency is not a live question until catch-up is in sight. Backfill proves a closed set of
signatures that can be differential-tested exhaustively; the tip cannot. hazync#139 (bigint2 ECDSA, 13.78× per
verify) was that trade; closed 2026-08-23: CORE ships without it, and it lives only in the GHOST build.

## 3. How it works

```
block ── cost packer ── chunks (independent, one card each) ── aggregate (join tree) ── block receipt
                                                                       └── absorbed into the spine [1..N]
```

- **Witnesses** come ready-made from the archive bridge, a full node that drives the accumulator once and emits a
  bundle per block, compactly encoded and de-duplicated per transaction. A prover needs no node and no replay.
- **A block** is split into chunks balanced by predicted cost; each proves independently; the aggregate re-checks
  each chunk's per-input binding digest and commits a 32-byte journal digest. A change that alters what was proven
  changes those bytes, which makes A/B tests trustworthy. Example, block 962,000 (8,006 inputs): 16 chunks,
  aggregate 323 segments, digest `4fb3e3c5…4656d`.
- **Segments** of one block can be proved on many machines (`seg-serve` / `seg-connect`); the join recursion is a
  tree. A worker sees only its segment, so it cannot forge, only fail to return. Measured: a near-tip block went from
  **4.9 h on one card to 100 min on three (2.92×**, ceiling 3.0×), byte-identical receipt each time.
- **Ranges** fold: `[a..b] + [b+1..c] → [a..c]`, same receipt size for 2 blocks or 200,000. The spine `[1..N]` grows
  by absorption. Spine receipt ~227 KB STARK, verifies in ms; a Groth16 wrap is a few KB.

**CORE vs GHOST.** Both commit the same digest as stock Core. Details: [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md).

| | CORE (ships since v0.21.0) | GHOST (experimental) |
|---|---|---|
| libsecp wNAF, GLV, ECDSA logic, checks | untouched | scalar-multiplication strategy replaced |
| cycles in Core's own code | ~all | ~22% |
| board | accepted | rejected (different `METHOD_ID`) |

v0.21.0 re-baselined to guest `37987b85` (2026-09-06), superseding `3867611d` (2026-09-04), because `0012`/`0013`
became unconditional. Block 962,000: 3,358 M cycles against stock's 13,748 M, journal byte-identical to a stock
control. The image id is reproducible: `reproduce/Dockerfile` rebuilds it byte for byte (`reproduce/METHOD_ID`), and
CI checks it.

**Check a proof (about 30 s):**

```bash
curl -fLO https://github.com/hazync/hazync/releases/latest/download/hazync-verify-x86_64-linux-gnu
chmod +x hazync-verify-x86_64-linux-gnu
curl -fLOJ https://api.hazync.org/api/spine/proof  # -J: lands as hazync-spine-1-<hi>.hzk
./hazync-verify-x86_64-linux-gnu hazync-spine-1-*.hzk
# >>> SNARK RANGE PROOF [1..N] VERIFIED — genesis-anchored
```

- The verifier is a 1.7 MB binary (Linux x86-64, glibc 2.34+; an `aarch64` build is published; on older distros use
  a container). `/api/proof/<height>` serves one block instead.
- Exit codes: `0` genesis-anchored, `2` valid mid-chain segment (expected for one block), `1` bad proof.
- Keep `-LO`: renaming breaks `sha256sum -c` against `SHA256SUMS.txt` ("no file was verified").
- The host (241 MB CPU, 410 MB CUDA, v0.21.4) proves and runs `verify-any proof.hzk` (prints `RANGE-OK`): the STARK
  checks and block *n* is a correct transition between its stated boundaries. Chaining to genesis is what the
  connected chain establishes.

**Repo map:** `prover/` guest and host · `verifier/`, `verifier-ffi/`, `verifier-wasm/` CLI, C ABI, browser ·
`patches/` every Core/libsecp change · `coreshim/` headers for the guest build · `scripts/` provisioning, benches,
gates · `reproduce/` the canonical `METHOD_ID` · `coordinator/` board and work distribution · `accumulator/`,
`coinbase-smt/`, `rangestate/` supporting crates · `audit-fuzz/`, `guest-pure-fuzz/`, `leaf-differential/`
adversarial harnesses · `docs/history/`, `experimental/`, `tasks/` the superseded record. Take numbers from `docs/`,
not `docs/history/`.

## 4. Where libzync fits

libzync is the new core library that is replacing the original guest; the original system's code stays in the repo.
Full account: [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md); plan: [`libzync/ROADMAP.md`](libzync/ROADMAP.md) §H.

- **Same judge.** Core v28.0's interpreter, sighash, serialisation and PoW and libsecp256k1 0.5.1 still decide every
  script and signature. libzync changes how the proof is organised: hinted signature checks confirmed by one batch
  equation per chunk, each transaction parsed and hashed once, an aggregate that never re-parses.
- **Coin accounting:** the coin log ([`libzync/COINLOG.md`](libzync/COINLOG.md)) proves every block; the chain state
  carries a counting trie root and a live-coin hash. The SwiftSync hints design (design B) was retired in E2
  (2026-11-05).
- **Handover and block forest** ([`libzync/HANDOVER.md`](libzync/HANDOVER.md)): a new image can continue a chain
  instead of re-proving from genesis (allowed for a rule change, never for a soundness fix); a tip proof can show an
  earlier block was in the chain.
- **zyncAPPs** ([`libzync/ZYNCAPPS.md`](libzync/ZYNCAPPS.md)): separate guests that verify a core receipt and answer
  one question (ownership, payment, supply). Design only; none built.
- **Proof statement and verifier:** [`libzync/SWIFTSYNC_PROOFS.md`](libzync/SWIFTSYNC_PROOFS.md) (name historical).
- **Networks:** mainnet and default signet.
- **Measured** (LIBZYNC.md, 2026-10-30): 8.2× fewer cycles than CORE `37987b85` on block 962,000 (409.5 M vs 3.36 G,
  round-8 image); mainnet tip blocks in 266–332 s on 4 GPUs (round-13 image). ROADMAP §H, 2026-11-06: block 962,000
  on one RTX PRO 6000 in **537 s** on image c14, the freeze candidate.
- **Backfill** (ROADMAP §H-B) returns to the original shape: independent anchored block proofs joined in one tree.
  Dev-mode 1..200 root byte-identical to the linear run; 8 real leaves on c14 joined, one 2^20 segment per join,
  6.4 s on a PRO 6000, root receipt 226 KB.
- **Status:** not yet frozen (the freeze is the user's call). No libzync proof from genesis on mainnet exists yet.
  The first independent review (2026-10-30) found soundness bugs, since fixed and retested.

## 5. Glossary

Where a doc and the code disagree, the code wins. ⚠ marks an overloaded or disputed term.

- **absorb** — fold the next range onto the spine: `[1..N] + [N+1..M] → [1..M]` (SPEC §10.1; host `extend-spine`;
  worker `hazync spine`). Block-map status 5 "anchored" = absorbed.
- **aggregate** — guest mode 5 (`aggregate()`), combining a block's chunk receipts into one block proof and
  re-checking each chunk's input binding digest. Host `prove-seg`, `agg-chunks`.
- ⚠ **anchor** — in [`SECURITY.md`](SECURITY.md) the trusted start of a chain proof (genesis or a block-hash
  checkpoint); in #277 ("anchor warp") a non-genesis checkpoint proved backwards from.
- ⚠ **audit #N vs round N** — review rounds 1–9 internal, 10–11 external AI-assisted (`history/SECURITY_AUDIT_LOG.md`);
  "audit #1–#5" is a separate series (#3 = 2026-08-03 BIP30 F-1, #5 = 2026-08-04). Round 8 =
  `history/AUDIT_2026-07.md`. The two numberings do not line up. No commissioned audit has happened.
- **beat** — `POST /api/beat`: refreshes `ranges.last_beat` on a held claim; ed25519 over `"<range>:<ts>"`, refused
  beyond `BEAT_SKEW` (120 s). The worker beats only after a segment finishes, so a hung prover's claim lapses.
  ⚠ Comments in `coordinator/server.py` (`CLAIM_TTL`, `do_POST`) and `cmd_run` still say there are no heartbeats.
- **bigint2** — risc0's 256-bit big-integer coprocessor. Substitutions `0005`–`0008`, `0014` are not canonical;
  `HAZYNC_BIGINT2_ECDSA=1` applies `0005` and changes the id.
- **block proof / leaf** — a range proof of width 1 (`prove-range-bridge`, or `prove-range` by replay). ⚠ "leaves" also
  means the Utreexo leaf count (`in_leaves`/`out_leaves`).
- **board** — the public coordinator at https://hazync.org/; also the `board` list in `/api/state`.
- **bridge** — archive node that replays the chain through the accumulator and writes a bundle per block to
  `HAZYNC_BRIDGE_OUT` (default `/root/bridge_bundles`) up to tip − `HAZYNC_BRIDGE_FINALITY` (default 100).
- **bundle** — `bundle_<n>.json`: in-boundary plus witness, enough to prove a block without replay. Served by
  `GET /api/witness/<n>` and `GET /api/witnesses`; falls back to legacy `block_<n>.json`.
- **channels (stock / CORE / GHOST)** — stock is the digest oracle, never on the board; CORE is canonical since
  v0.21.0 (`0001`, `0002`, `0012`, `0013`); GHOST is experimental, different id. ⚠ `history/CORE_VS_GHOST.md` §2
  (2026-08-30) also lists `0009`/`0010` under CORE; `provision-vps.sh` does not apply them.
- ⚠ **chunk** — a contiguous slice of a block's inputs, guest mode 4 (`chunk_prove()`); `HAZYNC_CHUNKS` sets the
  count (default 2), packed by predicted cost. SPEC §10.1 and `hazync spine` also call an absorbed range a chunk.
- **claim** — `POST /api/claim`: the earliest block neither proven nor live-claimed, width 1. Advisory: `submit`
  accepts any height; a per-claim `nonce` makes retries idempotent (#268). Releasing cancels nothing.
- **CLAIM_TTL / CLAIM_GRACE / CLAIM_MAX** — 3600 s from last beat (or claim); 600 s for a claim that never beat
  (#296); 86400 s hard cap.
- **coinbase SMT** — sparse Merkle tree of coinbase txid counts that closes BIP30 (Utreexo proves only membership)
  without the BIP34 height bound (#54). Root in `in_smt_root`/`out_smt_root`. Editing `coinbase-smt/src/roots.rs` or
  `bip30.rs` is a re-baseline.
- **contributor** — an ed25519 key (`hazync id`, under `$HAZYNC_HOME`, default `~/.hazync/key.hex`).
  `POST /api/rotate` moves attribution to a new key (both sign; history not rewritten, #113); off unless
  `ROTATE_ENABLED=1` (#311).
- **field backend** — patch `0012` (`field_bigint2`): libsecp field arithmetic on bigint2. See
  [`history/FIELD_BIGINT2_BACKEND.md`](history/FIELD_BIGINT2_BACKEND.md).
- **fold** — `[a..b] + [b+1..c] → [a..c]` when boundaries match in full; `range_work` sums (SPEC §10). Guest mode 7,
  host `fold-range`, worker `hazync fold`. The coordinator offers only aligned equal-width sibling pairs
  (`GET /api/foldable`), so folds build a tree.
- **frontier** — highest `hi` of the most-work seam-continuous chain of verified ranges from genesis; in `/api/meta`
  and `/api/state`. Runs ahead of the spine.
- **genesis-anchored** — a range with `lo == 1` whose in-boundary is genesis in full (SPEC §9,
  `RangeState::is_genesis_anchored`).
- **Groth16 SNARK wrap** — a succinct receipt compressed to Groth16 over BN254 (host `snark-wrap`; `verify-snark`
  re-applies the genesis pin); verification is three pairings. CPU only.
- **handle** — display label for a key, max `MAX_HANDLE` (48), HTML-stripped; default `ghost:<6 hex>`.
- **journal / RangeState** — the public output a receipt commits: `kind` (`0xC4A10006`), `lo`, `hi`, in- and
  out-boundary, `range_work`, `self_id`. Decoded positionally (SPEC §8); `scripts/check-rangestate.sh` keeps copies in
  step.
- **lift** — risc0 step turning a `SegmentReceipt` into a `SuccinctReceipt`.
- **lift_x hint** — patch `0013`: the host supplies pubkey Y and the guest checks `y² = x³ + 7` with libsecp
  arithmetic. See [`history/LIFTX_HINT.md`](history/LIFTX_HINT.md).
- **join / join tree** — risc0 step merging two adjacent succinct receipts; the vendored risc0 uses a balanced tree of
  depth `log2(N)` whose levels are published as work.
- **lineage** — every canonical id in order with risc0/rzup versions: `reproduce/LINEAGE.tsv`, from
  `scripts/lineage.sh` (`--check` in CI; needs a full clone).
- **maximal-Core** — as much of the result decided by Core's code as possible, with narrow substitutions at interfaces
  Core or libsecp already parameterise (`0002` is the precedent). ⚠ In `reproduce/METHOD_ID` (2026-07-26) it meant
  sourcing all constants from `chainparams.cpp`.
- **METHOD_ID / guest id / image id** — risc0 image id of the guest ELF; receipts verify against it and commit it as
  `self_id`. Canonical value in `reproduce/METHOD_ID`; `host method-id`; `GET /api/meta`.
- **po2 / seg_po2** — segment size exponent: `HAZYNC_SEG_PO2`, else 21 (CUDA) or 20; worker retries down to 18.
- **proof party** — the public effort to prove the chain from genesis ([`../CONTRIBUTING.md`](../CONTRIBUTING.md)).
- **range** — inclusive span `[lo..hi]`, written `lo-hi` or `N`.
- **re-baseline** — any change that moves the id, including line-moving comment edits in guest or `#[path]`-included
  files (panic metadata embeds line numbers). Old proofs stop verifying. `scripts/rebaseline-id.sh`,
  `scripts/check-guest-inputs.sh`.
- ⚠ **resolve** — discharging an assumption against a conditional receipt; `seg-serve` runs them itself by default
  since 2026-09-21 (#446), or sends them out with `HAZYNC_RESOLVE_LOCAL=0`. The chain is serial either way. Also
  risc0's `SegmentRef::resolve()`, which only loads a segment.
- ⚠ **segment** — a risc0 slice of at most `2^po2` cycles, proved independently then lifted and joined. In
  `/api/spine/segments` it means a run of spine blocks absorbed by one contributor.
- **seg-serve / seg-connect** — distributed proving of one block: `seg-serve` executes, publishes segments, joins,
  resolves and assembles (`HAZYNC_AGG=1` for the aggregate); `seg-connect <host:port>` is a stateless worker. Not the
  board coordinator.
- **spine** — the single genesis-anchored `[1..N]` with the largest N, advanced only by absorption; verified and
  stored but not built by the coordinator. `GET /api/spine`, `/api/spine/proof`. Run one spine worker fleet-wide.
- ⚠ **sponsor** — someone paying for a span to be proven (`/api/sponsor*`, [`COORDINATOR.md`](COORDINATOR.md)).
  Handles folding to `sponsor…` need a key in `sponsor_keys`; the bot (`coordinator/sponsor_bot.py`) submits as
  `SPONSOR: <name>`. The September docs disagreed on whether the bot exists; the code exists and had never run live.
- **STARK receipt / succinct** — risc0's native proof; composite = list of segment receipts, succinct = one
  fixed-size recursion-compressed STARK (chunk, aggregate and fold proofs are succinct).
- **straggler** — max ÷ mean chunk cost in a block; a block proves in its slowest chunk's time, which caps scaling.
- **Tier 0** — codegen speed with no fidelity cost: C `-O3` for libsecp, Rust fat LTO + `codegen-units = 1`,
  `ECMULT_WINDOW_SIZE` (`history/TIER0_RESULTS_2026-08-26.md`).
- **vrange** — a verified range row (`vranges` table): `lo`, `hi`, tips, boundary digests, `range_work`, key,
  handle. `GET /api/vranges`.
- **witness** — host-supplied, untrusted per-block input: header, transactions, prevouts, accumulator proofs,
  boundaries (SPEC §6). The guest derives what it can instead.
