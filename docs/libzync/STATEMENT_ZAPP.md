# Statement of balance zyncAPP

Status (15.11.2026): shape A is built and tested on branch `feat/zapp-statement` (`swiftsync-verify --statement`,
`zync-statement`, WASM export `verify_statement` behind a build feature). Shape B is a measured prototype in
`experimental/libzync-bench/statement/`; no image id is published for it.

A person or a business shows: **the coins at these scripts held X sat in total at block H, and whoever holds each
script signed this statement.** Anyone checks it with the statement file, the core proof and a verifier, offline
except for the one step every Hazync fact needs: comparing block H's hash with a source the checker trusts.

## 1. What a statement shows, and what it does not

Proven, by the core receipt and the checks in §4:

- the chain from genesis to H is valid Bitcoin under Core's rules (the core proof);
- block H is the block the statement names (its hash is the proof's tip; the checker compares it with the network);
- at H, each script held exactly the unspent count and value its balance fact states (the counting trie, `trie_root`);
- X is the sum of those values, and no script is counted twice;
- for each script, a valid BIP-322 signature exists over a message that names the network, the core image, H, the
  block hash, the exact script list, X, a purpose and a nonce.

Not proven:

- **Control is not exclusive control.** A signature shows that someone who could sign for the script signed. It
  does not show that nobody else can: a key held by an exchange, a co-signer, a thief or a backup service signs just as
  well. Two people can each produce a statement over the same coins. A statement is evidence of ability to sign, not of
  ownership in law.
- **Nothing after H.** The coins may be spent in the block after H. A statement is "as of block H". How old H may be
  is the checker's policy (§5).
- **When the signature was made** is not shown, except that it was made after the nonce existed: the message carries
  it. A key that has since leaked or been sold still signs today.
- **Spendability.** The balance counts every unspent coin at the script, including coinbase outputs not yet 100 blocks
  deep, and coins whose script needs more than one key (a 2-of-3 multisig counts once for the whole script).

## 2. Two shapes

| | A: statement file (built) | B: statement guest (prototype) |
|---|---|---|
| new guest | none | one zyncAPP image |
| what the checker sees | every script, its balance, each signature | the total, the number of scripts, a salted commitment to the script list, the purpose, the nonce |
| verifier | `swiftsync-verify --statement`, WASM `verify_statement` | the zyncAPP receipt against its image id, then the same core checks as ZYNCAPPS.md §4 |
| signature code | the rust-bitcoin project's `bip322` crate (CC0-1.0), outside the zkVM | in the guest (prototype: hand-written P2WPKH and P2TR, k256) |
| script types | everything the `bip322` crate verifies (§6) | what the guest implements (prototype: P2WPKH, P2TR key path) |
| cost to make | none: facts and signatures | a proof per statement (cycles in §7) |
| new trust | none beyond the core proof and the crate | the zyncAPP's source and image id, its reproducible build |

**Recommendation: A**, for now. It needs no new image, no proving and no new reviewed guest code; the signature checks
come from a maintained crate instead of our own. B only adds privacy, and a statement's main readers (an auditor, a
lender, a counterparty) usually want to see the scripts anyway. B is worth building when someone asks for the private
form; §7 gives its measured cost.

## 3. Encodings (A)

**Statement** (JSON, `swiftsync_verify::statement::Statement`):

```
{ "version": 1, "network": "mainnet", "core_id": "<64 hex: the image that proved the chain>",
  "height": H, "block_hash": "<64 hex, display order>", "purpose": "<text>", "nonce": "<text>",
  "total": X, "entries": [ { "fact": <TrieFact, kind "balance", at H, with expect>,
                             "format": "simple" | "full" | "legacy", "signature": "<base64>" }, ... ] }
```

Entries are in strictly increasing scriptPubKey byte order, so a script cannot appear twice. Purpose and nonce are 1
to 256 bytes with no control characters: a line break would let a purpose forge a line of the message a wallet shows.

**Message** (UTF-8, `\n` between lines, no trailing newline), what every script signs:

```
Hazync statement of balance v1
network: mainnet
core image: <core_id, lower case>
block: <H> <block hash, display order, lower case>
scripts: <number of entries> sha256 <script list hash>
total: <X> sat
purpose: <purpose>
nonce: <nonce>
```

The script list hash is SHA-256 over the scripts in entry order, each as CompactSize(length) followed by its bytes
(its serialisation inside a transaction). `zync-statement message` prints the text exactly.

**Signature formats**: `simple` (a BIP-322 simple signature: the witness, base64), `full` (BIP-322 full: the to_sign
transaction, base64), `legacy` (P2PKH only: Bitcoin Core `signmessage`, the BIP-137 format). A BIP-322 proof is
accepted only when it is valid with nLockTime 0 and nSequence 0 (BIP-322's "valid at time T and age S" for T = S = 0);
a proof valid only from a later time or age, or one the crate calls inconclusive, is refused.

## 4. Checks and soundness

`check_statement` (`swiftsync-verify/src/statement.rs`) runs only on a chain state whose receipt passed `verify`
(real proof, not a FakeReceipt, `RISC0_DEV_MODE` unset, the image, profile, anchor, block forest, time-too-new). Then:

1. `version` is 1; `network` is the proof's network (profile); `core_id` is the image that made the receipt
   (`self_id`); `height` is the proof's tip height; `block_hash` is the proof's tip hash.
2. Purpose and nonce are well formed and, when the checker gives `--purpose` and `--nonce`, equal to them.
3. Entries: 1 to 10,000, scripts strictly increasing.
4. Each fact is a `balance` fact with a stated answer, about its own script at H, and holds against the proof's trie
   root (`check_trie_fact`, the same check as `--trie`). Its answer (`never` = 0, or `<count>:<sat>`) is what is
   summed; the sum must not overflow and must equal `total`.
5. The message is rebuilt from the statement's fields; each entry's signature must verify for its script over it.

Why this is enough: every field the message names is either checked against the verified proof (network, core image,
H, block hash), recomputed (script list hash, total) or compared with the checker's own challenge (purpose, nonce). A
signature over the message therefore commits its signer to exactly this statement. Changing any field breaks every
signature; keeping the fields and swapping in a different fact fails step 4 (the fact's script is the signed script,
and a fact holds only for the answer the trie gives). The balance itself comes from the counting trie, whose root the
core proof commits and whose facts are refused for any single changed bit (`trie_fact` tests).

**Binding to the core across handovers.** The statement names the image that made the receipt, and the verifier
accepts only images it already trusts (its built-in table, or `--image` for one the reader rebuilt). A statement made
under an earlier image stays checkable while the verifier keeps that image: v0.26.0 keeps `8888d59c` next to
`46666b6f`, and the tests use `8888d59c`.

**A closed height.** With the coin log (profile bit 4) the trie is the coin set after every block, so the proof's tip
H is final for that proof: there is no hints file or epoch to close. A statement is therefore only ever about the
proof's tip, and a proof of a later tip needs a new statement.

**Block-hash binding and chain identity.** The proof shows a valid chain; that it is the network's chain is the tip
hash at H compared with a source the checker trusts (`--expect`, `--mempool`, `--bitcoin-cli`, or the page's
`expect`). Without it the verdict is at most DEMO, as for every fact.

**Replay.** A statement shown again under another purpose or nonce fails step 2. Rewriting the purpose to match
breaks every signature. Without `--purpose` and `--nonce` the facts and signatures still hold, but the statement may
be an old one shown again: the verdict is then at most DEMO and says why.

## 5. Using it

```sh
zync-statement draft --receipt <core receipt> --trie state_<H>.trie --purpose "<asked>" --nonce "<asked>" <scriptPubKey hex>... > s.json
zync-statement message s.json                      # the text to sign in another wallet
zync-statement sign s.json --keys <WIF file>       # or: zync-statement add-sig s.json <spk> simple|full|legacy <base64>
swiftsync-verify --expect <H>:<hash> --purpose "<asked>" --nonce "<asked>" --statement s.json <core receipt>
```

The checker chooses the purpose and nonce, gives them to the holder, and rejects any H further behind its own tip than
its policy allows (a lender might take 6 blocks, an annual audit a day). `zync-statement` signs P2WPKH, P2SH-P2WPKH
and P2TR (key path, no script tree) in BIP-322 simple and P2PKH in BIP-322 full, from WIF keys it never prints;
`add-sig` refuses a signature that does not verify. `zync-statement test-key <file>` makes a fresh key for tests.

The trie snapshot (`state_<H>.trie`) comes from a proving run at H; a mainnet holder needs a fact server that
keeps one (ROADMAP C1, `zync-serve`). The statement holds no secret: anyone can check or forward it.

## 6. Coins that cannot sign

The `bip322` crate (0.0.12) checks P2WPKH, P2SH-P2WPKH, P2TR key path, P2WSH and P2SH-P2WSH multisig, P2SH multisig
(full format) and P2PKH (full format or legacy). It has no script interpreter, so it calls anything else inconclusive,
and the statement refuses it:

- **P2PK** (the early coinbases, including block 9's coin the tests use): no address form, so the crate cannot
  take one. BIP-322's full format covers P2PK in principle (a scriptSig with one signature), with a verifier that
  runs the script.
- **Bare multisig** and non-standard scripts: the same.
- **P2SH or P2WSH of any script other than multisig**, and **taproot script paths**: inconclusive.
- **A P2SH coin whose redeem script the holder has lost** cannot sign at all, under any format.

Such coins can still be shown to exist with a balance fact; they cannot be put in a statement. A holder can move them
to a script that signs, at the cost of a transaction.

## 7. Shape B: measured

The prototype guest (`experimental/libzync-bench/statement/methods/guest`) verifies the core receipt (`env::verify`,
an assumption), decodes the ChainState and makes the ZYNCAPPS.md §4 checks, checks each balance fact against
`trie_root` with the shared `libzync::trie_fact` source, builds a message with a salted commitment
`sha256(salt || script list hash)` in place of the list hash, checks each BIP-322 simple signature
(`bip322_lite.rs`, P2WPKH and P2TR), and commits the core fields, the total, the number of scripts, the commitment,
the purpose and the nonce. `bip322_lite.rs` is cross-checked natively against the `bip322` crate
(`statement-bench selftest`): every signature the crate makes verifies, and a changed message, key or signature does
not; BIP-322's official P2WPKH and P2TR vectors verify.

Execute mode on CPU (cycles; no proof was made), scripts alternating P2WPKH and P2TR:

MEASUREMENTS

What B would still need: the core's libsecp in place of k256 (ZYNCAPPS.md §7), P2PKH and multisig, a published image
id from a reproducible build, a verifier entry point, and a decision on what the commitment hides: the number of
scripts is public, and without the salt a guessed script list can be tested against the commitment.

## 8. Tests

- `cargo test --release statement` (swiftsync-verify): 6 tests on the real mainnet 1..200 proof (image `8888d59c`,
  `vectors/mainnet_1_200_8888d59c.rcpt`) and its block-200 trie snapshot (`vectors/mainnet_200_8888d59c.trie`, root
  equal to the proof's). No key of ours was paid in blocks 1..200, so the positive case on the real trie is a zero balance
  for two fresh keys; non-zero cases (P2WPKH, P2TR, P2SH-P2WPKH, P2PKH full, P2PKH legacy) use a trie built in the test
  under the real proof's state with its trie root replaced. Refusals: tampered total, tampered fact answer, wrong
  height, block, core image and network, another purpose, another nonce, a purpose rewritten to match, another key's
  signature, a signature over another message, a fact relabelled to a script it does not prove, a duplicate script,
  unsorted scripts, a history fact, a fact with no answer, a line break in the purpose, a wrong format, no entries,
  another version, a time-locked BIP-322 proof, block 9's real P2PK coin (no address form), and a FakeReceipt.
- `swiftsync-verify/test_statement.sh`: the CLI end to end on the real proof, 16 cases (17 with
  `STATEMENT_WASM=<.wasm>`, which runs `swiftsync-verify-wasm/test-statement.mjs`, 9 cases, through a WASM build
  with feature `statement`). Results: `swiftsync-verify/test_statement.out`.
- The `bip322` crate's own suite (77 tests, including BIP-322's official vectors) passes at 0.0.12.

## 9. Open

- The name. This document says "statement of balance"; the owner decides the product name.
- P2PK: the early coins are the ones people most often want to show, and the crate cannot check them. A P2PK check
  is a few lines on top of the crate's to_spend / to_sign (one ECDSA signature over the legacy sighash); hand-written,
  so it needs review and BIP-322-style vectors of its own.
- Freshness as a verifier flag (`--max-age <blocks>` against the checker's node or mempool.space), rather than policy.
- Per-coin statements (BIP-322 "proof of funds", which signs the coins' outpoints) instead of per-script: stronger
  (the coins themselves are signed for) but needs a coin fact with its creating transaction for every coin.
- Whether the WASM build should carry the statement check by default: it doubles the .wasm (1,248,921 to 2,554,120
  bytes raw, measured).
