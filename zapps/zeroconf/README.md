# `zapps/zeroconf`: the zero-confirmation zyncAPP's check

A proof that transaction T is valid for the block after a proven tip, and that every coin it spends is unspent at
that tip. A merchant learns it before T confirms. ⚠ Valid now, not final: a conflicting transaction can still confirm
first. A T that spends an unconfirmed parent is refused by the guest (its coin is not in the coin log).

| part | where |
|---|---|
| the check library `zapp_zeroconf` (`verify_zeroconf`: the engine proof under the pinned key, the caller's txid, the core policy) | `src/` |
| an engine proof with real children (regtest, tip 110) and its refusals | `tests/engine.rs`, `tests/data/engine/` |
| the guest | `prover/zyncapps/methods/zeroconf` |
| the prover (`zyncapp prove-zeroconf --stream`, then `hzengine prove`) | `prover/zyncapps/host`, engine/host |
| the regtest fixture, Core's verdicts, the guest's refusal cases | `engine/zapps/zeroconf/` |

Engine (OpenVM) proofs only: the module is new with the engine (engine/VERIFIER.md). No CLI or WASM build yet.

```sh
cargo test --release
```
