# `swiftsync-verify`: check a libzync chain proof, and facts about the chain it proves

A standalone verifier for libzync proofs. No prover, no Bitcoin Core build, no GPU, no chain data. It checks that the
receipt proves the chain from genesis (with its coin log, and with the product image its counting trie and live-coin
hash), and checks facts given with it against the proven state: coins (`--coin`, `--journey`) and counting-trie facts
(`--trie`: a coin unspent / spent by / absent under its script, a script's balance or complete history). A trie fact
carries its claimed answer in `expect` (`absent`, `unspent:<sat>`, `spent:<sat>:<height>:<tx>:<input>`; for balance and
history `never` or `<count>:<sat>`), which `zync-path trie` writes and the check must reproduce: a false claim fails.
Without `expect` the fact is a question, answered in the output line (the C ABI refuses those). The name is historical:
the SwiftSync hints file (design B) was retired in E2 and no proof takes one.

```sh
cargo build --release --manifest-path swiftsync-verify/Cargo.toml
swiftsync-verify/target/release/swiftsync-verify --network signet --product proof.rcpt
swiftsync-verify/target/release/swiftsync-verify --network mainnet --product --mempool --trie fact.json proof.rcpt
```

| verdict | exit | meaning |
|---|---|---|
| `SWIFTSYNC_VERIFY_OK` | 0 | a genesis-anchored proof with enough work: the chain printed, and every fact given, hold |
| `SWIFTSYNC_VERIFY_DEMO` | 2 | every check passed, but from a state given with `--anchor`, below the minimum work, or facts without the tip compared with the network |
| `SWIFTSYNC_VERIFY_FAIL` | 1 | anything else, including unreadable input |

**What it checks:**
1. the receipt is a real proof (no dev-mode `FakeReceipt`, `RISC0_DEV_MODE` unset) of image M, and its journal is
   that image's `ChainState` (kind, version, `self_id`);
2. the profile the image commits (scripts checked or not, network, coin log, trie, live-coin hash); a field of a
   feature the image lacks is zero;
3. `anchor_id` is the network's genesis state (or the `--anchor` state, giving at most DEMO), and the lineage;
4. the block forest has one leaf per block; `time-too-new` on this machine's clock;
5. chain identity, with `--expect <h>:<hash>`, `--mempool` or `--bitcoin-cli "<command>"`: the proven tip is the
   network's block at that height. Without one the tip and work are printed and the verdict line says so.

**Trie facts.** `zync-path trie coin|balance|history <state_<h>.trie> <scriptPubKey hex> [<txid> <vout>]` writes one
from a prover run's trie snapshot; `zync-serve` serves them at `/trie/coin/<spk>/<txid>/<vout>`, `/trie/balance/<spk>`
and `/trie/history/<spk>`. Each is checked against the verified state's trie root (`libzync::trie_fact`).

**Proof bundles.** `swiftsync-verify [--network ...] bundle <receipt> <out.sspf>` writes one file holding the
receipt and a header naming network, image, height, tip, work and trie root (`docs/libzync/PROOF_BUNDLE.md`, version
2). A bundle is accepted wherever a receipt is; its header is checked against the verified result and never trusted.

**Test vectors** for this or any other verifier: `vectors/` (refusals only until the first GPU proofs of the frozen
image; `python3 vectors/run_vectors.py <verifier>`).

**The image id is the trust root.** The built-in ids (`src/lib.rs`, `IMAGES`) are pinned at the image freeze. A
reader should rebuild M from the published commit and pass it with `--image <hex>` rather than trust this file.
`--image` is printed on the first line of the output.

**Tests:** `cargo test --release` (genesis work, the empty-SMT root and genesis `anchor_id`, coin and trie facts),
and `bash swiftsync-verify/test.sh` from the repo root (end-to-end refusals; output in `test.out`).
