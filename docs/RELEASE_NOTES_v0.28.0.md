> [!IMPORTANT]
> **DRAFT, 22.11.2026. Not released.** Before the cut, fill every `⏳ PLACEHOLDER` and delete this box:
> - ⏳ PLACEHOLDER: the release date (dd.mm.yyyy) in the CHANGELOG entry.
> - ⏳ PLACEHOLDER: #788 (strict compact sizes), #790 (url 2.5.9) and #789 (ooga-land, README, CONTRIBUTING) are written up here as merged; they were still open on 22.11.2026.
> - ⏳ PLACEHOLDER: the release's asset hashes (from its signed `SHA256SUMS.txt`) in "Asset hashes" below.
> - ⏳ PLACEHOLDER: the GPU gate's cards and times (Linux pre-Blackwell, Linux Blackwell, Windows) in "Proved on real GPUs" below.

**The zyncAPP parsers read transactions as Bitcoin Core does, and the board learns to follow the tip.** The core image is unchanged (mainnet `46666b6f`, signet `01a80f17`, headers `de54b94f`): the board keeps its chain and nobody re-proves a block. Three zyncAPP images move (hints, utreexo, utreexo-tip), so **receipts made under their old images stop verifying**, and six WASM modules have new hashes.

```
mainnet      46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c   (unchanged)
signet       01a80f17469dfd75869534ffdbf3105e55438afc77329e818eca7044b4129be7   (unchanged)
headers      de54b94f40d750c746aafd8f306988791a074e424e81e411702312d4c528bf07   (unchanged)
hints        11c72c44dd47a28f26a7d9b284fa10897ac85b08ecf20a34e87f5d120995ecfb   (was 1894f12e)
utreexo      00b2d1e856b5ac55dc330da5b0d7143ccda357edb18d61a69a808999b2d2d4c6   (was 27bea7da)
utreexo-tip  5ef26dfeb987a448f6921e8e7546620488a24b90f69c342b1837409d3c6c023d   (was ab6f7a26)
```

All are in `reproduce/LIBZYNC_IDS`, from `reproduce/libzync.Dockerfile`, each from two agreeing from-scratch builds (#788). RISC Zero stays at 3.1.0.

## What breaks

- **Old hints, utreexo and utreexo-tip receipts.** A receipt made under `1894f12e`, `27bea7da` or `ab6f7a26` is refused by v0.28.0's `zapp-hints`, `zapp-utreexo`, their WASM modules and the utreexo follower. I am not keeping the old ids as accepted ids. The only real receipts under them that I know of are the repo's own vectors, and they are re-proved (below). If you made one yourself, prove it again with v0.28.0.
- **New WASM hashes.** A page that pins a module by its SHA-256 must take the new one. The core module is the release asset `hazync-verify.wasm`; the others are built with each module's `build.sh` in the published build image.

  | module | v0.27.1 | v0.28.0 |
  |---|---|---|
  | core `hazync-verify.wasm` | `0c8cf026bd088d049d0c6becdfefdc4b21cfec75a66c48c391e1d115325e1917` | `893aa98ba0b93308f752193d02c72b1fd2072b9e1c967a419fb9b7c433768028` |
  | `zapp-headers.wasm` | `bc0fa13a46028b047468834f56bebcc8a9ab3b23537dd54952e164334785f5b7` | `a7ed0ac848139b56966d9e4a27f82c7fe59b0a5903b15b7602f084b0aacb3c1d` |
  | `zapp-hints.wasm` | `906676442ce95e34cedbca36c65ccafe6f3e64067d3f39e37e4f3ed2bc3023e7` | `1c7f050c4401a303e826654dea1a6bb42d2f71d94069e74fddff8565feacb584` |
  | `zapp-utreexo.wasm` | `d66746848b925e2d5c0e15814238ecbb53cb4459f76d79d617c52d66b141d7ef` | `9f1d14c2cf33b1388d7b46551c079e72e6e591c7a5f435091ae9b54e927602d7` |
  | `zapp-deposit.wasm` | `f7d7f49b66b2dbc513cda81712699223ba531c792b456a9cce9fe9cdba59962b` | `4f48016f78472f996b917ba8a70b52ae06633558d38911a39ba1cf833d9de60c` |
  | `zapp-statement.wasm` | `81e98ce904ae63169f2e92c1d39784013104e583b33a5d29b86bebe0c95c1a2f` | `a6911de661c2775cc48e8ef7f727877fe77e30bc4e53e458e7274081862bb486` |
  | `zapp-assume.wasm` | `582dc8fa…` | unchanged |
  | `zapp-liveness.wasm` | `38eb760e…` | unchanged |

  The core module moves although the core image does not: it reads transaction facts with the parser this release makes strict. Each module's README pins its hash. The core and `zapp-headers` modules still ship together on a page that checks headers.
- **Malformed transactions.** A transaction whose compact sizes v0.27.1 read leniently (below) is now refused everywhere Hazync parses one. Bitcoin Core refuses the same bytes, so no valid Bitcoin transaction is affected.
- **Not affected:** core receipts (`46666b6f`, `01a80f17`), header-chain receipts (`de54b94f`), the board's chain, and the assume, liveness, statement and deposit zyncAPPs' receipts (they have no guest of their own).

## Strict compact sizes in every transaction parser

Every parser now reads compact sizes with Bitcoin Core's rules (#788, which contains #774):

- **Shortest form only.** `fd 01 00` is refused, as Core's `ReadCompactSize` refuses it ("non-canonical"). Before, it parsed as 1, and because the stripped form re-encodes the counts, such a transaction got the txid of other bytes.
- **At most `MAX_SIZE`** (0x02000000). This also ends a wasm32 truncation: `as usize` read 2^32 + 3 as 3.
- **Every length within the bytes left,** with no `as usize` truncation.
- **No superfluous witness.** A witness section in which every stack is empty is refused, as Core's "Superfluous witness record".

`libzync::facts::varint` and `facts::len_varint` are now the only readers of these rules. The core facts parser, the hints and utreexo block walks, the utreexo tip step and the header-chain zyncAPP's transaction check all use them. That is why the three zyncAPP images move. The core guest does not include this parser, so its image does not.

**Tested** (#788):

- every parser has a `compact_sizes_are_cores` test, and its MAX_SIZE + 1 case fails if MAX_SIZE is removed;
- 9 of 9 lenient-reader controls are caught;
- the hints, headers, utreexo, deposit, statement, assume and liveness `test.sh` all pass.

**Re-proved vectors,** real CPU proofs from the pinned container, no dev mode:

- hints `mainnet-2` and `mainnet-3`;
- utreexo `mainnet-2`, `mainnet-3` and `mainnet-3-split`;
- the utreexo tip receipt `mainnet-tip-103`, on the core spine 1..4,790, in 5,180.9 s of CPU. That spine receipt is now kept in the repo (`zapps/utreexo/vectors/core-spine-4790.receipt`), so the vector can always be proved again.

**Reviews:** #774 had two Fable reviews and the delta one more. None found an input wrongly accepted or refused; every item they raised is fixed.

## Following the tip

### The board follows the tip (D1)

Until now the board proved only the blocks one prep run (the board pass) cut, up to a fixed height. From this release the coordinator also follows the **paper prep**, which follows the chain tip (#779; `docs/libzync/REORG.md` §19):

- **Two runs, one board.** Blocks from X = 974,937 come from the tip run, blocks below X from the board pass. A verification sees exactly the two states it reads. A check confirms both runs agree where they meet.
- **Reorgs per run.** Reorg records and acks are kept per run. A reorg marker fences a run above its fork: nothing above it is served, claimed or credited until the rewind. Automatic rewinds stay **off**, as in v0.27.0.
- **A bounded leaf window.** Tip bundles live in a window of 12,000 leaves. When it is full, prep pauses; nothing is deleted.
- **Tip blocks first** (`TIP_CLAIM_FIRST`, on in the shipped drop-in). Tip blocks go first to workers that can prove one: workers that say so (`HAZYNC_TIP=1`) or whose tip block has verified. Everyone else gets the backfill first, so a small card is not handed a mainnet-size block.
- **Safety:**
  - the prep runner refuses to clear stores that hold state files (`LZ_RUN_DRYRUN=1` shows what a start would take);
  - the coordinator refuses to start with a missing or unreadable blocks directory;
  - the worker retries a 503 and keeps an unsent receipt for its next run.
- **Checks that page:** a misconfigured X, D1 not engaging once prep passes X, tip blocks not proved for 24 h, the tip run stalled, a stale node tip.
- **Reviewed** in three independent rounds (11, 10 and 7 items), each item fixed with a test whose control fails without it. The tip-run end-to-end harness (`experimental/libzync-bench/prep/reorg_e2e_tip.py`) passes 29 checks, and its control fails.

⏳ PLACEHOLDER: whether D1 is deployed on the board at release time. It is deployed by `coordinator/deploy/RUNBOOK.md`, "Following the tip", which includes the paper prep's one restart.

### zAPPs at the tip (Z2–Z5)

The zAPP facts the site serves can now come from a height near the tip, not only from the board pass's stores (#779; `docs/libzync/ZAPP_FOLLOWER.md` §14):

- a read-only reader of a prep run's disk trie at pinned roots (libzync, `reader` feature; never in the core verifier);
- `zync-serve --tip`, answering at any listed height (`?at=<h>`, default the newest);
- a tip follower (`hazync-lz-zapp-follow-tip`) that keeps two heights, pins on a fall-back and restarts the server safely.

The website's part (sending the height) is merged after the relay switch.

### utreexo tip mode: a production witness source

v0.27.0's utreexo tip mode used an in-memory host forest, fine for tests but not for the mainnet tip. v0.28.0 adds Hazync's own disk forest (#783; `docs/libzync/UTREEXO_ZAPP.md` §10.5.1, §10.9.1):

- **Why not a utreexod bridge:** a pruned utreexod refuses to serve proofs; unpruned, it needs a second copy of the chain, its own proofs and a sync from genesis. The forest needs no new dependency and grows as the follower steps.
- **Trust is the same either way: none.** The guest rebuilds every leaf and opens every deletion against the previously proven roots, so a lying source can only stall the follower.
- **`ArenaForest`:** a port of rustreexo 0.7.0's `MemForest` onto an arena, byte-identical to `MemForest` on 160 random histories. Saved as a checksummed file whose inner hashes are recomputed on load.
- **`zyncapp utreexo-tip-prove --source forest`** (with `--cache-required`).
- **The follower** keeps forests. After a reorg it falls back to the newest kept forest at or below the target and replays forward. It refuses a replay from genesis above `GENESIS_REPLAY_MAX` unless allowed, and saves crash-safely.
- **Checks:** a stall and heap check (`hazync-check-utreexo-follow`), orphan pruning, and a CI job (`utreexo-follower`) in which 10 controls each fail first at their own check.

**Measured** (#783):

- mainnet steps 1..100,000: the forest gives the same witness bytes and roots as the native replay at every step, BIP30 heights included;
- a step from 91,880 to 100,000 has 38,151 deletions and takes 0.69 s;
- a real-proof follower run passes 34 of 34.

Memory and load time at the mainnet tip are not measured yet. Nothing of it is deployed.

## Also since v0.27.1

- **Windows GPU gate:** the segment size follows the card's memory, from the app's own table, and the script warns below 3 GB free (#785). In the v0.27.1 gate a 4 GB GTX 1050 Ti ran out of memory at the default size and out of disk at 1.4 GB free; at segment size 18 it proved block 170 in 296.6 s.
- **Release notes:** the repo's copies of the v0.26.1 and v0.27.0 notes open with the same warning as their GitHub releases: use v0.27.1 (#786).
- **Unit-drift check:** two false alarms fixed: several assignments on one `Environment=` line, and unit files outside the three directories it searched (#787).
- **Dependencies:** url 2.5.9 for RUSTSEC-2026-0592, host side only; no image moves (#790).
- **Docs:** the Ooga Booga Land page names the Hazync release key and the board's 16.11.2026 restart; `CONTRIBUTING.md` has v0.27.1's Linux host sizes (#789).

## Proved on real GPUs

⏳ PLACEHOLDER: the gate's result for this release's exact binaries (`scripts/release-gpu-gate.sh`): blocks 170 and 200,000 on one pre-Blackwell and one Blackwell card, each card's name, prove times and the spend, and block 170 on the Windows card.

## Upgrading

**Provers on the board** (the Windows app, `hazync-worker`, the Linux hosts): the core image is unchanged, so your v0.27.1 host keeps making valid proofs.

- Update when you can: v0.28.0's worker retries a 503 and keeps an unsent receipt for its next run, instead of losing it.
- If your card can prove a mainnet-size block, set `HAZYNC_TIP=1` on the worker to get tip blocks first.
- The Windows app updates itself.
- On Linux with a GPU, do not go back to v0.26.1 or v0.27.0 (see their notes).

**zAPP provers** (hints, utreexo, utreexo-tip): build the CLIs and `zyncapp` from the v0.28.0 tag, and prove again any receipt you made under the old images. Old receipts are refused (above).

**Verifiers:**

- **The command line:** take v0.28.0's `hazync-verify`. It checks core receipts exactly as before and refuses the malformed transaction facts above.
- **zAPP CLIs:** build them from the v0.28.0 tag; the v0.27.x ones refuse the new hints and utreexo receipts.
- **In a browser:** load the new WASM modules and pin their new hashes (the table above). The core `hazync-verify.wasm` is the release asset; hazync.org moves to it with this release.

**Board operators:** the coordinator changes (D1, tip first, the new checks) are deployed by the RUNBOOK's "Following the tip". Do not deploy them without it: it includes the paper prep's one restart, with pre-checks and a fallback.

## Asset hashes

⏳ PLACEHOLDER: the sha256 of every asset, from this release's signed `SHA256SUMS.txt` (14 assets). `hazync-verify.wasm` must be `893aa98ba0b93308f752193d02c72b1fd2072b9e1c967a419fb9b7c433768028`.
