# `swiftsync-verify`: check a libzync chain proof, and facts about the chain it proves

A standalone verifier for Hazync's chain proofs: engine (OpenVM v2.1.0) proofs, in the `HZE1` file
(engine/VERIFIER.md). Releases ship it as `hazync-verify-x86_64-linux-gnu` and `hazync-verify-aarch64` (and, built
from `../swiftsync-verify-wasm`, as `hazync-verify.wasm`). No prover, no Bitcoin Core build, no GPU, no chain data.
It checks that the proof shows the chain from genesis, with its coin log, counting trie and live-coin hash, and
checks facts given with it against the proven state: coins (`--coin`, `--journey`) and counting-trie facts
(`--trie`: a coin unspent / spent by / absent under its script, a script's balance or complete history). A trie fact
carries its claimed answer in `expect` (`absent`, `unspent:<sat>`, `spent:<sat>:<height>:<tx>:<input>`; for balance and
history `never` or `<count>:<sat>`), which `zync-path trie` writes and the check must reproduce: a false claim fails.
Without `expect` the fact is a question, answered in the output line (the C ABI refuses those). The name is historical:
the SwiftSync hints file (design B) is retired and no proof takes one.

The RISC Zero chain's proofs are checked with v0.27.1's verifier, the last RISC Zero release.

```sh
cargo build --release --manifest-path swiftsync-verify/Cargo.toml
V=swiftsync-verify/vectors/mainnet_1_200_engine
swiftsync-verify/target/release/swiftsync-verify --network mainnet --image $(cat $V.image_id) --vk $V.vk --vk-sha256 <the key file's pin> $V.hze
swiftsync-verify/target/release/swiftsync-verify --network mainnet --image <hex> --vk key.bin --vk-sha256 <the key file's pin> --mempool --trie fact.json proof.hze
```

The first line is the engine chain's proof of mainnet 1..200: `SWIFTSYNC_VERIFY_DEMO`, exit 2 (a valid chain from
genesis, below Core's minimum chainwork and the pinned checkpoint).

| verdict | exit | meaning |
|---|---|---|
| `SWIFTSYNC_VERIFY_OK` | 0 | a genesis-anchored proof with enough work: the chain printed, and every fact given, hold |
| `SWIFTSYNC_VERIFY_DEMO` | 2 | every check passed, but from a state given with `--anchor`, below the minimum work or the checkpoint, or facts without the tip compared with the network |
| `SWIFTSYNC_VERIFY_FAIL` | 1 | anything else, including unreadable input |

**The image and its key.** No engine image is published yet: the records go in at the Stage 4 freeze. Until then every
call gives `--image <hex>` (the image id, SHA-256(exe || vm)) and `--vk <key file> --vk-sha256 <the key file's pin>` (its verifying key). OpenVM's
verifier trusts the key it is given, so the key is pinned by YOU: `--vk-sha256` is the key file's SHA-256 from the
image's record or your own build (`vk.sha256`), a key that does not hash to it is refused, and every check runs under
exactly those bytes. The image id alone adds no security: it does not bind the key, so a key that came with the
proof, unpinned, proves nothing (review R1-6 #1, 27.11.2026). Without them the verifier fails with "no engine image is published for <network> yet: give the image id
and its key". A reader rebuilds the image and takes the key from that build.

**What it checks:**
1. the file is an engine proof that verifies against image M under the pinned key (the STARK, the image id from the
   key, the public values = SHA-256 of the journal), and its journal is that image's `ChainState` (kind, version,
   `self_id`);
2. the profile the image commits (scripts checked, network, coin log, trie, live-coin hash, node format 2);
3. `anchor_id` is the network's genesis state (or the `--anchor` state, giving at most DEMO), and the lineage;
4. the block forest has one leaf per block; `time-too-new` on this machine's clock;
5. chain identity, with `--expect <h>:<hash>`, `--mempool` or `--bitcoin-cli "<command>"`: the proven tip is the
   network's block at that height. Without one the tip and work are printed and the verdict line says so.

**Trie facts.** `zync-path trie coin|balance|history <state_<h>.trie> <scriptPubKey hex> [<txid> <vout>]` writes one
from a prover run's trie snapshot; `zync-serve` serves them at `/trie/coin/<spk>/<txid>/<vout>`, `/trie/balance/<spk>`
and `/trie/history/<spk>`. Each is checked against the verified state's trie root (`libzync::trie_fact`).

**The header-chain zyncAPP** is its own module (`../zapps/headers`: the CLI `zapp-headers`, the WASM module and
`zapp-headers.js`; docs/libzync/ZYNCAPPS.md §9), on this crate as a library. This verifier checks core chain proofs only.

**Proof bundles.** `swiftsync-verify [flags] bundle <proof.hze> <out.sspf>` writes one file holding the proof and a
header naming network, image, height, tip, work and trie root (`docs/libzync/PROOF_BUNDLE.md`, version 2). A bundle is
accepted wherever a proof is; its header is checked against the verified result and never trusted.

**Test vectors** for this or any other verifier: `vectors/` (16 cases on the engine chain's proof of mainnet 1..200;
`python3 vectors/run_vectors.py <verifier>`).

**Tests:** `cargo test --release` (genesis work, the empty-SMT root and genesis `anchor_id`, coin and trie facts, the
engine proof of block 200), and `bash swiftsync-verify/test.sh` from the repo root (19 end-to-end cases; output in
`test.out`).
