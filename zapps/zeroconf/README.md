# `zapps/zeroconf`: the zero-confirmation zyncAPP's check

A proof that transaction T is CONSENSUS-valid for the block after a proven tip (Core's transaction rules and
scripts, within one block's weight and sigop-cost limits), and that every coin it spends is unspent at that tip. A
merchant learns it before T confirms.

- Not relay policy: a consensus-valid T can still fail to propagate (non-standard, below the minimum relay fee). The
  check reports whether T is within Core's relay limits for size and sigops; it checks no other policy.
- Not final: a conflicting transaction can still confirm first.
- About the tip you give: pass your CURRENT tip (`--expect`, `--bitcoin-cli`, `--mempool`). A proof against an older
  block of your own chain is about that block, and misses every spend since.
- `--wtxid` also checks the witness of the bytes you hold (the txid alone fixes T but not its witness).
- A T that spends an unconfirmed parent is refused by the guest (its coin is not in the coin log).

| part | where |
|---|---|
| the check library `zapp_zeroconf` (`verify_zeroconf`: the engine proof under the pinned key, the caller's txid, the core policy) | `src/` |
| an engine proof with real children (regtest, tip 110) and its refusals | `tests/engine.rs`, `tests/data/engine/` |
| the guest | `prover/zyncapps/methods/zeroconf` |
| the prover (`zyncapp prove-zeroconf --stream`, then `hzengine prove`) | `prover/zyncapps/host`, engine/host |
| the regtest fixture, Core's verdicts, the guest's refusal cases | `engine/zapps/zeroconf/` |

| the CLI `zapp-zeroconf` (`--vk` with its pin `--vk-sha256`, `--image`, `--core-image`, `--txid`, `--wtxid`, the chain-identity flags; exit 0 OK, 2 DEMO, 1 FAIL) | `cli/` |

Engine (OpenVM) proofs only: the module is new with the engine (engine/VERIFIER.md). No WASM build yet.
The statement's kind is its own, 0x5A5A0001, since 27.11.2026; before, the guest committed the coin zAPP's 0x5A430002
(a proof of that image is refused: tests/data/engine/old_kind).

```sh
./test.sh      # the library on the real proof, then the CLI on it (15 cases)
```
