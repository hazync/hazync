# Fresh cost model (experimental)

A from-scratch look at what proving Bitcoin *must* cost, built without reference to how
Hazync does it today. Goal (agreed 23.11.2026):

- answer **any question about anything in any block**, genesis to tip;
- **keep up with the tip within minutes**;
- **Bitcoin Core's code stays the consensus judge**; well-specified primitives
  (signatures, SHA-256) may be proven by separate, purpose-built components.

## Step 1: measure the work (this directory)

`scan/` measures, from a real unpruned mainnet node, the primitive work a validator must do:

| script | what |
|---|---|
| `stats_scan.py` | `getblockstats` for every height (exact txs / inputs / outputs / bytes / UTXO growth) |
| `detail_scan.py` | decodes sampled blocks (every 250th + the last 1,008) and counts ECDSA / Schnorr signatures, SHA-256 compressions by purpose, RIPEMD-160, script bytes, UTXO ops |
| `aggregate.py` | scales the sample to the whole chain per 10,000-block era using the exact totals; profiles the tip block |

Signature counts are **signatures present** (DER-shaped / 64-65 byte taproot items);
`ecdsa_cms_upper` adds CHECKMULTISIG's worst-case extra attempts. SHA counts are
compression-function calls computed from preimage sizes, not executed.
