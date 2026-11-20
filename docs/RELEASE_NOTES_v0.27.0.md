**zyncAPPs on top of the chain proof, and the board learns to survive a reorg.** v0.27.0 adds six zyncAPPs, extends the header-chain one, adds safe
reorg handling for the board, a stricter verifier and a Windows app that updates itself. The mainnet core image stays
**`46666b6f`**: the board keeps its chain and nobody re-proves. RISC Zero stays at 3.1.0.

```
mainnet      46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c   (unchanged)
signet       01a80f17469dfd75869534ffdbf3105e55438afc77329e818eca7044b4129be7   (unchanged)
headers      de54b94f40d750c746aafd8f306988791a074e424e81e411702312d4c528bf07   (was 2db34e12)
hints        1894f12eaef5ea50e649b56d0e8fe6afd5b59deb99331e2a5ad49f1a52eb0e8a   (new)
utreexo      27bea7da7c9c2c528a0c9b6e6d22f5d06c5b2c87bcd9d58c360ffecaf5424fb9   (new)
utreexo-tip  ab6f7a26fe4cb13446c3d0b3ee352e6bf3508a897e0363a0a2ef6a559d51353e   (new)
```

All are in `reproduce/LIBZYNC_IDS`, from `reproduce/libzync.Dockerfile`, each reproduced by two builds. The assume,
liveness, statement and deposit zyncAPPs have no guest of their own, so no image id.

## Header chain: every block hash in a Merkle Mountain Range

The header-chain zyncAPP proved that a run of mainnet headers is a valid chain. Its state now also carries an MMR of
every block hash, so a verifier holding the proof can check two more things:

- **a block:** "block X at height h is on this proven chain", with an MMR path;
- **a transaction:** with the block's 80-byte header, the raw transaction and its merkle branch, "transaction T is in
  a block on this chain".

How it works:

- **One MMR source** (`mmr.rs`), compiled by the guest, the verifier and the host's path builder, so they cannot drift.
- **The 64-byte merkle ambiguity is closed.** The raw transaction must hash to the txid and must not be 64 bytes. The
  block's coinbase and its branch pin the tree's depth, so a branch of any other length is refused.
- **Strict input** (#765): the include JSON and the `/tx` answer take only their documented keys, each once. A misspelt
  or `null` `txid`/`tx` is refused, not read as a block-only check.
- **CLI:** `zapp-headers ... --include <h>:<hash> --path <file> [--tx <tx.json>]`. **Host:** `zyncapp
  headers-mmr-path`.

**The whole chain, proved again** (15.11.2026, one RTX PRO 6000, image `de54b94f`): genesis to 975,278 in one receipt.

- 975,279 headers, 4,656,795,655 cycles, 4,739 segments. The old image took 2,350,725,289 cycles: the MMR about
  doubles the work per header.
- Proof: 99.5 min. Proof share of the pod bill: $4.63 ($5.16 with setup and build).
- Receipt: 227,474 bytes. Its tip hash, chainwork and MMR root equal the full node's. One bit flipped is refused.

**A verdict is OK only when the tip was checked** against a source you trust (`--expect`, `--mempool`,
`--bitcoin-cli`; in the browser, `expect`). Otherwise it is **DEMO**, and so is a block or transaction check on it.

## Block check: an untrusted path service

`hazync-headers-api` serves a block's MMR path (`/path`) and what the transaction check needs (`/tx`), so a browser
does not hold a million block hashes. It is **untrusted by design**: the browser checks every answer against the peaks
the proof commits to. A lying service can refuse to answer, never make a block or transaction look included.

- It serves the tip the follower **published**, and fails closed (503) while its hashes do not end on it.
- Read-only, rate-limited, sandboxed; reachable only from the web box.

## SwiftSync hints zyncAPP (new)

SwiftSync is Ruben Somsen's protocol; the hints file format is Robert Netzke's. This zyncAPP proves that **a hints file
for height H is exactly right**, so a node can check a downloaded file before it spends a sync on it. (A wrong file is
already safe in SwiftSync; the proof saves the wasted sync.)

- **What it proves:** the file's SHA-256 is the hints file for block H of the chain the core receipt proves, plus the
  SHA-256 of every block's undo hash from 1 to H. The guest rebuilds every coin-log leaf and both forests and requires
  them to equal the core proof's.
- **Formats:** format 1 follows the BIP's test vectors' bit order, format 2 the `hintsfile` crate 0.2.0's. They
  disagree; we have asked the maintainers which is right.
- **Cost, measured** (CPU execute, not a proof): 27.4 M cycles at H = 1,999, 135.0 M at 10,000, 2.39 G at 100,000.
  No mainnet-tip figure.
- **Tests:** 26 of 26 at each of those three heights, including tampering refused by the guest.
- **Real proofs only at H = 2 and 3** (225,202 bytes each; no spends). Treat it as early.
- **Entry points:** `zapp-hints`, a C ABI for a node patch to call before `-utxohints`, and WASM (`verifyHints`).

## Five more zyncAPPs

Each is its own module: the core verifier and the core image do not change. Each had an independent review on
15.11.2026 (no soundness break found). Test counts are from 16.11.2026, on a laptop.

- **assume** (#740): two checks on a core proof, no new guest.
  - `zapp-assume assumevalid` prints the proven tip as `-assumevalid=<hash>`.
  - `zapp-assume assumeutxo` checks a Core `dumptxoutset` snapshot against the proven coin set. `hash_serialized_3`
    and MuHash equal Core's at heights 200 and 100,000.
  - Tests: 33/33, WASM 14/14, C ABI 6/6.
- **liveness** (#741): an outpoint is unspent (or spent) at H on a fully validated chain, e.g. a Lightning funding
  output.
  - Proofs 333 to 734 bytes; 18,904 bit-flips refused.
  - The privacy variant is an experiment, measured in execute mode only.
- **statement** (#742): a holder signs a statement over their coins at H with BIP-322, checked against the core
  proof's balance facts. Its own optional WASM. The privacy guest (shape B) is a prototype, not published.
- **deposit** (#743): transaction T is in block h, buried N deep, on a chain where every block up to the core tip is
  fully validated. A fake deposit in an invalid block is refused, which SPV cannot do.
  - By default the deposit's own block must be fully validated (`--min-validated-depth 1`).
  - A Groth16 wrap was measured (88 to 97 s, about 4.6 KB). Not the default: it needs a trusted setup.
- **utreexo** (#744): proven Utreexo roots for Floresta's assumeutreexo.
  - **Base:** walks every block and proves the forest roots at H. Byte-identical to Floresta's own code at H = 1,999,
    10,000 and 100,000 (executed). Real CPU proofs at H = 2 and 3.
  - **Tip mode (new):** Floresta's developers answered in getfloresta/Floresta#1472 (16.11.2026): they want the
    latest height, a few blocks below the tip. A step proof starts from the previous receipt's roots and applies the
    new blocks with rustreexo 0.7.0 itself, so the newest receipt alone proves everything.
  - Steps executed to 100,000, across the BIP30 blocks, equal Floresta's roots at every height; 21 tampered inputs
    refused by the guest, each with its reason.
  - One real step proof, 3 to 103: 4,945.9 s on a loaded shared CPU box (not a benchmark), 226,138 bytes.
  - The follower (`hazync-utreexo-follow`) passed 31 of 31 end to end with three real CPU proofs, including a reorg
    fall-back. Nothing is deployed.
  - **For now the core proof bounds H, not the tip:** the core spine was at 3,836 on 18.11.2026.

## Reorg handling, P1 to P6

The board is about to follow the chain tip, so some blocks it proved will be orphaned. **Automatic rewinds stay OFF**
(`HAZYNC_REORG_AUTO` unset): every fork below prep's tip stops for a person, who recovers with one command. No guest
change.

- **P1, undo records:** one per block, kept for J = 2,016 blocks. About **1.05 MB per block** at the tip, so the
  automatic window W = 288 blocks is 302 MB.
- **P2, detect and stop:** the fetcher checks prep's tip against the node every poll, at any depth. A fork below
  prep's tip stops the fetcher and prep (exit 4) and alerts.
- **P3, prep's rewind:** `host libzync-prep-rewind <out> <blocks> [--allow-deep]` undoes the stores to the fork and
  moves everything above it to `orphaned/<epoch>/`. A rewind that dies is finished from `out/REWIND`.
- **P4, the coordinator's rewind (new):** prep writes a reorg record and waits for the coordinator's ack.
  - The coordinator moves orphaned rows and proofs aside (never deletes them), drops the spine to a kept head, voids
    claims above the fork and acks.
  - A proof verified across a rewind is refused (409 `reorged`) and not counted against the new chain's block.
  - Contributors keep credit for orphaned proofs, shown as "n orphaned".
  - New: `/api/meta.reorg`, `/api/reorg/<E>`, `server.py --reorg-to F`, and a backstop that pages
    (`hazync-lz-check-reorg`) when a verified range is not on prep's chain.
- **P5, the followers (new):**
  - Offsite copies upload only proofs at least 6 blocks below the node's tip. A deeper reorg is recorded with markers;
    nothing offsite is deleted.
  - The zAPP follower rebuilds from genesis when the spine drops below its stores.
  - The website shows a banner while the board rewinds, and lists orphaned proofs on a block's page (hazync-web
    `web/reorg-banner`, not deployed yet).
- **P6, the end-to-end harness:** fetcher, prep and coordinator against a fake node, reorgs at depth 1, 2, 12 and 288,
  mid-prove and mid-prep. 18.11.2026, after the third review: W = 12 PASS (78 checks), W = 288 PASS (81); the control
  fails 15. `rewind_test.sh` 57/57.
- **Review:** five rounds on P4–P6 (#764), each finding fixed with a test that fails without it.

**What turning automatic rewinds on requires** (REORG.md §17):

- P5 deployed, after the reviews (with P4 alone, an orphaned proof copied offsite would block the new one for good).
- The coordinator deployed first, with the ack directory both units name (`/var/lib/hazync-lz/reorg-acks`).
- Then `HAZYNC_REORG_AUTO=1` on the board pass and its fetcher. It is a deployment decision, not a code change.
- The coordinator follows one prep run, the board pass. The paper prep following the tip (D1) is not built.

## A stricter verifier

From a fuzzing round on 17.11.2026 and review (#758, #762):

- **No more aborts on bad input.** An unreduced BabyBear word, a journal that is not whole words, or
  `RISC0_DEV_MODE` set used to kill the C ABI's host, trap WASM or panic the CLI. Each is now a FAIL.
- **Trailing bytes are refused.** A real proof with bytes appended used to verify DEMO.
- **Facts are strict** (#762): unknown fields in a fact and unknown top-level keys in the facts JSON are
  refused. A misspelt `tries` used to skip every fact and could return OK.
- **The prover host too** (#763): receipts, journals and worker frames it did not make are decoded strictly, so
  `verify-any` refuses a trailing byte and an unreduced word is a refusal, not an abort.
- Every zAPP that calls the core verifier gets these fixes. No guest change.

## The Windows app

Discovered during Windows app usage on 18–19.11.2026 (#761, #770, #771):

- **Stale prover or client, named and fixed in one click.** The app remembers paths, and an install replaced only its
  own copies, so an old prover made the wrong image. Startup and Start now name the prover and client in use, the
  image the prover makes and the board's, and offer the switch.
- **The client never replays from block 1 on a libzync board.** It says "update Hazync" instead of failing every
  claim with "HTTP 404".
- **In-app update.** It checks GitHub at startup and every 6 hours. On your click it downloads the installer and
  `SHA256SUMS.txt`, verifies the PGP signature against the key pinned in SECURITY.md, checks the installer's hash, and
  runs it. No downgrades; never while work is running. The installer is not Authenticode-signed: the signed manifest is
  the trust anchor.
- **Automatic segment size follows the card** (#770). On a 4 GB GTX 1050 Ti, Automatic ran out of memory on block
  4401 because it tried the host's default first. It now picks the size from the detected card's memory.
- **A live resource panel** (#771): GPU, CPU and RAM while proving, and each block's peak GPU memory.

## Board and operations

- **RunPod REST v2** (#759, #769): the sponsor bot, tip runs and fold rentals move off RunPod's GraphQL API, which
  RunPod is retiring. Every pod a run creates carries the run's name prefix, and it counts as gone only once a listing
  no longer shows it. The account balance stays on GraphQL (v2 has no balance call). The zAPP regtest GPU script uses the same client (#769).
- **Docs from the stranger test of 18.11.2026** (#766, hazync-web #62; v0.26.1, CPU): 14 defects fixed in CONTRIBUTING, README, SECURITY,
  the explainer and the worker's messages, and the website's contribute, home and devs pages.

## Build and CI

- **Faster PR CI** (#760): the 3-hour soundness pass is skipped on a pull request that cannot affect it. Over the
  last 60 merges, 23 would have been light (about 20 min). Main, nightly and manual runs still run everything.
- **Actions pinned to commit shas**, with Dependabot keeping them current (#757). A tag can be moved; a sha
  cannot.
- **Duplicate keys in a workflow are refused** (#767): GitHub drops the whole workflow on one, and #724 and #731 had each
  added the same job.

## The browser verifiers: one module per zyncAPP

`verifyHeaders` moves out of the core verifier into its own module, so the core stays the core. **The website must
ship them together**: a page that loads the new core module and still calls `verifyHeaders` on it finds no such
function.

- `hazync-verify.wasm` (core): sha256 `0c8cf026…`.
- `zapp-headers.wasm`: `bc0fa13a…`. It needs `expect` for an OK verdict.
- `zapp-hints.wasm`: `90667644…`.
- `zapp-assume.wasm` `582dc8fa…`, `zapp-liveness.wasm` `38eb760e…`, `zapp-statement.wasm` `81e98ce9…`,
  `zapp-deposit.wasm` `f7d7f49b…`, `zapp-utreexo.wasm` `d6674684…`. Full hashes in each module's README.

Each builds byte for byte in the published build image, the same on two machines.

## What changes for you

- **Provers:** nothing. The core image is unchanged and the board keeps its chain.
- **Verifiers:**
  - Core receipts are checked as before, but a receipt with trailing bytes, or facts JSON with unknown keys, is now
    refused.
  - Header-chain receipts are `de54b94f`'s; give the tip you trust, or the verdict is DEMO.
  - Anyone producing facts: a new fact field now breaks older verifiers, so ship verifiers first.
- **Board operators:**
  - Install prep's new units and `alert.conf` drop-ins, then restart prep once (undo records exist only from then on).
    Recovery after a stop: `libzync-prep-rewind` (REORG.md §11).
  - With P4: the ack directory in both units, the new `hazync-lz-check-reorg` timer. Offsite units now need a fresh
    `/var/lib/hazync/node_tip`, or they upload nothing and fail.
  - With #759: replace the hand-installed `backstop_spend.py` on hazync-proof with the ported copy.
- **Website:** ship the core module with `zapp-headers`, pass `expect` to `verifyHeaders`, and deploy the reorg
  banner with P5.
- **Windows app users:** install v0.27.0 by hand once. From then on the app offers updates itself.

## Not in this release

- **The paper prep following the tip (D1):** the coordinator follows only the board pass.
- **zAPP tip mode Z2 to Z5:** facts at the tip read from the tip run's stores. Z1 (pinned roots) is in; today's zAPP
  follower rebuilds instead.
- **Automatic reorg rewinds:** built, but OFF (above).
- **A production witness source for utreexo tip mode:** designed (a utreexod bridge node preferred, or a disk forest),
  not built. Today's host keeps the forest in memory: fine for tests, not for the mainnet tip.
