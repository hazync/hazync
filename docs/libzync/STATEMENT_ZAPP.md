# Balance statement zyncAPP

Status: shape A, built and tested 15.11.2026 and released in v0.27.0 (#742, 20.11.2026) on RISC Zero, engine (OpenVM)
core proofs only since 27.11.2026 (no engine image is published yet, so the checker names the core image and its key),
is its own zAPP module,
`zapps/statement` (ZYNCAPPS.md §9): a library on top of the core verifier, one CLI `zapp-statement` (the checker, and
the producer as subcommands `draft`, `message`, `sign`, `add-sig`, `test-key`), and its own WASM module (`zapps/statement/wasm`). The core verifier (`swiftsync-verify`,
`swiftsync-verify-wasm`) carries no statement code. Shape B is a measured prototype in
`experimental/libzync-bench/statement/`. Its image ids appear in `RESULT_statement.txt` as measurement records only:
no verifier exists for them, and none may be written (§8).

A person or a business shows: **the coins at these scripts held X sat in total at block H, and whoever holds each
script signed this statement.** Anyone checks it with the statement file, the core proof and the checker, offline
except for the one step every Hazync fact needs: comparing block H's hash with a source the checker trusts.

## 1. What a statement shows, and what it does not

Proven, by the core proof and the checks in §4:

- the chain from genesis to H is valid Bitcoin under Core's rules (the core proof);
- block H is the block the statement names (its hash is the proof's tip; the checker compares it with the network);
- at H, each script held exactly the unspent count and value its balance fact states (the counting trie, `trie_root`);
- X is the sum of those values, and no script is counted twice;
- for each script, a valid BIP-322 signature (or, for P2PKH, a BIP-137 one) exists over a message that names the
  network, the core image, H, the block hash, the exact script list, X, a purpose and a nonce.

Not proven:

- **Control is not exclusive control.** A signature shows that someone who could sign for the script signed. It
  does not show that nobody else can: a key held by an exchange, a co-signer, a thief or a backup service signs just as
  well. Two people can each produce a statement over the same coins. A statement is evidence of ability to sign, not of
  ownership in law.
- **Nothing after H.** The coins may be spent in the block after H. A statement is "as of block H". How old H may be
  is the checker's policy (§5).
- **When the signature was made** is not shown. If the checker's nonce is unpredictable (random, made when the
  checker asks), the signature was made after the checker asked; a guessable nonce shows nothing about time. A key
  that has since leaked or been sold still signs today.
- **Who is showing it.** A statement is not bound to the person presenting it. Someone challenged by a checker can pass
  the purpose and nonce to a real holder under false pretences and show the holder's statement as their own (§4,
  relay). The purpose is the defence, if the signer reads it.
- **Spendability.** The balance counts every unspent coin at the script, including coinbase outputs not yet 100 blocks
  deep, and coins whose script needs more than one key (a 2-of-3 multisig counts once for the whole script).

## 2. Two shapes

| | A: statement file (built) | B: statement guest (prototype) |
|---|---|---|
| new guest | none | one zyncAPP image |
| what the checker sees | every script, its balance, each signature | the total, the number of scripts, a salted commitment to the script list, the purpose, the nonce |
| checker | `zapp-statement`, WASM `zapp-statement.wasm` | the core proof against its image id and pinned key, then the statement (§4) |
| signature code | the rust-bitcoin project's `bip322` crate (CC0-1.0), outside the zkVM; P2PK hand-written on top of it (§7) | in the guest (prototype: hand-written P2WPKH and P2TR, k256) |
| script types | what the `bip322` crate verifies, and P2PK (§6, §7) | what the guest implements (prototype: P2WPKH, P2TR key path) |
| cost to make | none: facts and signatures | a proof per statement (cycles in §8) |
| new trust | none beyond the core proof and the crate | the zyncAPP's source and image id, its reproducible build |

**Recommendation: A**, for now. It needs no new image, no proving and no new reviewed guest code; the signature checks
come from a maintained crate instead of our own (except P2PK, §7). B only adds privacy, and a statement's main readers (an auditor, a
lender, a counterparty) usually want to see the scripts anyway. B is worth building when someone asks for the private
form; §8 gives its measured cost.

## 3. Encodings (A)

**Statement** (JSON, `zapp_statement::Statement`):

```
{ "version": 1, "network": "mainnet", "core_id": "<64 hex: the image that proved the chain>",
  "height": H, "block_hash": "<64 hex, display order>", "purpose": "<text>", "nonce": "<text>",
  "total": X, "entries": [ { "fact": <TrieFact, kind "balance", at H, with expect>,
                             "format": "simple" | "full" | "legacy", "signature": "<base64>" }, ... ] }
```

No other field is read, at any level (statement, entry, fact): a field not listed here is refused, since it would be
in no signed message. Entries are in strictly increasing scriptPubKey byte order, so a script cannot appear twice. Purpose and nonce are 1
to 256 bytes of printable ASCII (0x20 to 0x7e), with no leading or trailing space: a line break, a Unicode line or
paragraph separator or a bidirectional control would let a purpose show a wallet's user a line the message does not
say. They are compared as given, never trimmed (CLI and WASM alike). `core_id` and `block_hash` are 64 lower-case hex
digits, refused otherwise before they are compared or printed (a line break in either could forge a verdict line).

**Message** (UTF-8, `\n` between lines, no trailing newline), what every script signs:

```
Hazync balance statement v1
network: mainnet
core image: <core_id>
block: <H> <block hash, display order>
scripts: <number of entries> sha256 <script list hash>
total: <X> sat
purpose: <purpose>
nonce: <nonce>
```

The script list hash is SHA-256 over the scripts in entry order, each as CompactSize(length) followed by its bytes
(its serialisation inside a transaction). `zapp-statement message` prints the text exactly.

**Signature formats**: `simple` (a BIP-322 simple signature: the witness, base64), `full` (BIP-322 full: the to_sign
transaction, base64; the only format for P2PK), `legacy` (P2PKH only: Bitcoin Core `signmessage`, the BIP-137 format). A BIP-322 proof is
accepted only when it is valid with nLockTime 0 and nSequence 0 (BIP-322's "valid at time T and age S" for T = S = 0);
a proof valid only from a later time or age, or one the crate calls inconclusive, is refused.

## 4. Checks and soundness

`check_statement` (`zapps/statement/src/lib.rs`) runs only on a chain state whose core proof passed
`swiftsync_verify::verify`
(the engine proof under the image and its pinned key, profile, anchor, block forest, time-too-new). Then:

1. `version` is 1; `network` is the proof's network (profile); `core_id` is the image that made the core proof
   (`self_id`); `height` is the proof's tip height; `block_hash` is the proof's tip hash.
2. Purpose and nonce are well formed and, when the checker gives `--purpose` and `--nonce`, equal to them.
3. Entries: 1 to 10,000, scripts strictly increasing.
4. Each fact is a `balance` fact with a stated answer, about its own script at H, and holds against the proof's trie
   root (`swiftsync_verify::check_trie_fact`, the same check as `swiftsync-verify --trie`). Its answer (`never` = 0, or `<count>:<sat>`) is what is
   summed; the sum must not overflow and must equal `total`.
5. The message is rebuilt from the statement's fields; each entry's signature must verify for its script over it.

Why this is enough: every field the message names is either checked against the verified proof (network, core image,
H, block hash), recomputed (script list hash, total) or compared with the checker's own challenge (purpose, nonce). A
signature over the message therefore commits its signer to exactly this statement. Changing any field breaks every
signature; keeping the fields and swapping in a different fact fails step 4 (the fact's script is the signed script,
and a fact holds only for the answer the trie gives). The balance itself comes from the counting trie, whose root the
core proof commits and whose facts are refused for any single changed bit (`trie_fact` tests).

**Binding to the core across handovers.** The statement names the image that made the core proof, and the verifier
accepts only an image it trusts: today the one the reader gives with `--image` (and its key, `--vk`); once engine
images are published, its built-in table too. A statement made under an earlier image stays checkable while the
verifier is given that image.

**A closed height.** With the coin log (profile bit 4) the trie is the coin set after every block, so the proof's tip
H is final for that proof: there is no hints file or epoch to close. A statement is therefore only ever about the
proof's tip, and a proof of a later tip needs a new statement.

**Block-hash binding and chain identity.** The proof shows a valid chain; that it is the network's chain is the tip
hash at H compared with a source the checker trusts (`zapp-statement --expect`, `--mempool`, `--bitcoin-cli`, or
the page's `expect`). Without it the verdict is at most DEMO, as for every fact.

**Replay.** A statement shown again under another purpose or nonce fails step 2. Rewriting the purpose to match
breaks every signature. Without `--purpose` and `--nonce` the facts and signatures still hold, but the statement may
be an old one shown again: the verdict is then at most DEMO and says why.

**Relay.** The nonce stops an old statement being shown again; it does not stop a new one being passed along. If B
challenges M, M can hand B's purpose and nonce to a real holder A ("sign this for my audit") and show A's statement to
B. The checks cannot tell: A really signed. The defence is the purpose: it should name both parties, the one asking
and the one who claims to hold the coins ("B's credit check of M, 15.11.2026"), and the signer must read it before
signing. A, reading "of M", refuses. A wallet that signs without showing the message gives no such protection.

A `holder:` line in the message would put the claimed holder in a field of its own, which a wallet could show
prominently and a checker could compare with the person it asked. I would add it in a version 2 message, not now:
version 1's message stays exactly as it is (a change would break every statement signed under it), and the purpose
already carries the same words. A v2 would be a new `version`, a new header line, and both read side by side.

## 5. Using it

```sh
zapp-statement draft --receipt <core proof.hze> --image <core id> --vk core.vk --vk-sha256 <the key file's pin> --trie state_<H>.trie --purpose "<asked>" --nonce "<asked>" <scriptPubKey hex>... > s.json
zapp-statement message s.json                      # the text to sign in another wallet
zapp-statement sign s.json --keys <WIF file>       # or: zapp-statement add-sig s.json <spk> simple|full|legacy <base64>
zapp-statement --image <core id> --vk core.vk --vk-sha256 <the key file's pin> --expect <H>:<hash> --purpose "<asked>" --nonce "<asked>" --bitcoin-cli "<cmd>" --max-age 6 <core proof.hze> s.json
```

`--max-age <blocks>`: block H must be at most that many blocks behind the network's tip, which `--bitcoin-cli`
(`getblockcount`) or `--mempool` (`/blocks/tip/height`) gives (all sources given must agree; `--expect` gives no tip,
so `--max-age` with only `--expect` fails). Without it the OK line says H's age was NOT checked: the coins may have
moved in any block after H.

`zapp-statement` exits 0 `ZAPP_STATEMENT_OK`, 2 `ZAPP_STATEMENT_DEMO` (the statement holds, but the chain is below
Core's minimum work, the tip was not compared with a source the checker trusts, or no purpose and nonce were given;
the line says which), 1 `ZAPP_STATEMENT_FAIL`. Every line it prints is escaped (line breaks and other control or
format characters as `\n`, `\u{2028}`, ...): text from the statement, or a library's error that echoes it, cannot print
a line of its own, such as a forged verdict. Each flag is taken once; a repeat is refused. The checker is its own binary rather than a `swiftsync-verify`
subcommand so that the core verifier carries no zAPP code; it verifies the core proof with the same library function.

The checker chooses the purpose and nonce (an unpredictable nonce, a purpose naming both parties, §4), gives them to
the holder, and sets `--max-age` to its policy. `zapp-statement` signs P2WPKH, P2SH-P2WPKH and P2TR (key path, no script tree) in BIP-322 simple
and P2PKH and P2PK (in the key's own form, compressed or uncompressed) in BIP-322 full, from WIF keys it never prints;
`add-sig` refuses a signature that does not verify. `zapp-statement test-key <file> [--uncompressed]` makes a fresh key
for tests.

In a browser, the page loads `zapp-statement.wasm` with `zapp-statement.js` (`loadStatementVerifier`,
`verifyStatement({ network, receipt, statement, expect, purpose, nonce, maxAge, tipHeight, imageHex, vk, pin })`); all
of these but the core proof (`receipt`, the `.hze` file's bytes) and the statement are the page's, never the sender's
(tipHeight: the network's tip as the page fetched it; imageHex, vk and pin: the core image, its key file and the key's
SHA-256, which the page is published with).
`age_checked` in the answer is false until maxAge and tipHeight are given. Only the statement page loads it; the core `hazync-verify.wasm` is unchanged. Built
byte-reproducibly by `zapps/statement/wasm/build.sh` (the core's flags, `scripts/wasm-build.sh`: Rust 1.99.0, staged
sources, remapped paths; it compiles C, secp256k1-sys, so in the pinned container `reproduce/wasm.Dockerfile`, clang
18.1.3): 2,493,761 bytes raw, sha256 `b14ce3a0c1620a7e86cd4a7f534e273d4453e57de25d0170abbfb40088badaa8` (the same on a
laptop and on hazync-coord, 16.11.2026, with the review fixes of that day: the image resolved by zapps/corecheck and an
exact image pin; before them `5a284b26…`, and with a host clang 14 `510c205b…`; the module no longer links zapps/common, its CLI is the crate `zapps/statement/cli`; the sha `zapps/statement/test.sh` checks is the one in
`wasm/README.md`; before the review fixes of 15.11.2026 `01408a2c…`).

The trie snapshot (`state_<H>.trie`) comes from a proving run at H; a mainnet holder needs a fact server that
keeps one (ROADMAP C1, `zync-serve`). The statement holds no secret: anyone can check or forward it.

## 6. Coins that cannot sign

The `bip322` crate (0.0.12) checks P2WPKH, P2SH-P2WPKH, P2TR key path, P2WSH and P2SH-P2WSH multisig, P2SH multisig
(full format) and P2PKH (full format or legacy). It has no script interpreter, so it calls anything else inconclusive.
P2PK is checked by this zAPP's own code (§7). Refused:

- **Bare multisig** and non-standard scripts: no address form, so the crate cannot take them.
- **P2SH or P2WSH of any script other than multisig**, and **taproot script paths**: inconclusive.
- **A P2SH coin whose redeem script the holder has lost** cannot sign at all, under any format.
- **A P2PK coin whose key is lost** (most 2009 coinbases): the fact holds, no signature can be made.

Such coins can still be shown to exist with a balance fact; they cannot be put in a statement. A holder can move them
to a script that signs, at the cost of a transaction.

## 7. P2PK

BIP-322 covers any script through its full format; the `bip322` crate takes only addresses, and P2PK has none. So
P2PK (`<33- or 65-byte key> OP_CHECKSIG`) is checked by `zapps/statement/src/p2pk.rs`, the only hand-written signature
code in this zAPP, kept to what the crate does not provide:

- **to_spend** is BIP-322's with the P2PK script as the challenge. It is built here because the crate's builder takes
  an address; a test checks that it gives the crate's txid, and that the crate's to_sign on it gives the crate's to_sign
  txid, for every script the crate takes (P2PKH, P2SH-P2WPKH, P2WPKH, P2TR) and three messages.
- **to_sign** is the crate's own `create_to_sign` on that to_spend. The signature must be exactly that transaction
  (version 0, one input spending to_spend:0 with nSequence 0, nLockTime 0, no witness, one 0-sat OP_RETURN output)
  with a scriptSig of exactly one push.
- **The required rules of BIP-322** for this script: SIGHASH_ALL only; LOW_S; STRICTENC (strict DER; the key
  compressed 0x02/0x03 or uncompressed 0x04, hybrid 0x06/0x07 refused); MINIMALDATA (the push minimally encoded);
  CLEANSTACK (one push, nothing else). The signature is verified by libsecp (through the `bitcoin` crate) over the
  legacy sighash of input 0, with the P2PK script as scriptCode (no OP_CODESEPARATOR can occur in it).
- **Stricter than BIP-322 in one place:** a scriptSig that is not a single push (for example a NOP before the
  signature) can still satisfy the script, but this checker has no interpreter and refuses it. BIP-322 lets a validator
  without a full interpreter stop at "inconclusive"; a statement refuses both. A time-locked proof (version 2) is
  refused, as for every format.

What it trusts: libsecp's ECDSA and the `bitcoin` crate's legacy sighash and transaction encoding, both the same code
the crate's P2PKH check uses; and the about 90 lines of p2pk.rs that assemble them, which need review before a release. Tests (§9)
cover both key forms, a changed message, signature byte, sighash type (0x02, 0x03, 0x81), high S (n − s of a valid
signature), padded DER, a non-minimal push, two pushes, a witness, a lock time, version 2, another key's signature,
the two P2PK forms' signatures swapped, a hybrid key, and the real block-9 P2PK coin (its fact holds; a signature by
our key filed under it is refused).

## 8. Shape B: measured (RISC Zero, a dated record; not ported to the engine)

The prototype guest (`experimental/libzync-bench/statement/methods/guest`) verifies the core receipt (`env::verify`,
an assumption), decodes the ChainState and makes the ZYNCAPPS.md §4 checks, checks each balance fact against
`trie_root` with the shared `libzync::trie_fact` source, builds a message (header `Hazync private statement of balance v1`, the prototype's text when measured) with a salted commitment
`sha256(salt || script list hash)` in place of the list hash, checks each BIP-322 simple signature
(`bip322_lite.rs`, P2WPKH and P2TR), and commits the core fields, the total, the number of scripts, the commitment,
the purpose and the nonce. The bench flag is committed first in the journal (the second word, after the kind): `true`
means `env::verify` was skipped, so that journal proves nothing about any chain. `bip322_lite.rs` is cross-checked natively against the `bip322` crate
(`statement-bench selftest`): every signature the crate makes verifies, and a changed message, key or signature does
not; BIP-322's official P2WPKH and P2TR vectors verify.

Execute mode on CPU, 15.11.2026 (user cycles, no paging or padding; no proof was made). Scripts alternate P2WPKH
and P2TR. `real`: the real mainnet 1..200 receipt (`8888d59c`) as the guest's assumption and its real block-200 trie
(zero balances: no key of ours is in it). `synth`: a trie built by the bench with 262,144 other scripts plus ours,
two coins each, under the guest's bench flag (no `env::verify`; a measurement, never a statement).
Raw lines: `experimental/libzync-bench/statement/RESULT_statement.txt`.

**These images must never get a verifier.** Image `74698c86…` (the first measurement) skipped `env::verify` under the
bench flag without committing it: a receipt of that image can carry any total, for any chain. The bench now commits
the flag (BENCH_IMAGE_LINE), but it is still a measurement guest: a statement guest, if one is built, is a new image
with no bench path at all.

| run | scripts | user cycles | per script | signatures | balance facts | input read | segments |
|---|---:|---:|---:|---:|---:|---:|---:|
| real | 1 | 716,836 | 716,836 | 318,236 | 11,975 | 264,240 | 1 |
| real | 10 | 3,700,501 | 370,050 | 2,985,856 | 113,589 | 478,441 | 4 |
| real | 100 | 33,732,103 | 337,321 | 29,877,041 | 1,107,530 | 2,624,973 | 35 |
| synth | 1 | 729,830 | 729,830 | 314,328 | 23,897 | 277,459 | 1 |
| synth | 10 | 3,926,478 | 392,648 | 2,994,883 | 221,200 | 596,268 | 5 |
| synth | 100 | 36,113,714 | 361,137 | 29,911,816 | 2,254,968 | 3,832,556 | 38 |
| synth, 1,048,576 others | 100 | 36,489,914 | 364,899 | 29,901,017 | 2,433,866 | 4,040,657 | 38 |

Signatures (k256, P2WPKH and P2TR mixed) are about 300,000 cycles each: 76 % to 81 % of the total at 10 scripts,
83 % to 89 % at 100. A balance fact is about 11,000 cycles on the real block-200 trie and about 22,000 to 24,000 on the
synthetic ones; quadrupling the other scripts (262,144 to 1,048,576) added about 1,800 cycles per fact. The fixed part
(the ChainState checks, the message, the commit) is under 120,000 cycles. "Input read" is risc0 serde reading the
inputs word by word (the journal and every byte of the proofs and witnesses); `read_slice` would cut most of it, as it
did for the core's aggregate. These are user cycles: resolving the core receipt (the `env::verify` assumption) is a
recursion step at proving time, not counted here and not measured.

What B would still need: the bench path removed, the core's libsecp in place of k256 (ZYNCAPPS.md §7), P2PKH, P2PK
and multisig, the 65-byte P2TR 0x00 refusal (the prototype accepts only 64 bytes or 65 ending 0x01 already), a
published image id from a reproducible build, a verifier entry point, and a decision on what the commitment hides: the number of
scripts is public, and without the salt a guessed script list can be tested against the commitment.

## 9. Tests

- `cargo test --release` in `zapps/statement`: 20 library tests and 1 engine test (`tests/engine.rs`) on the engine
  chain's proof of mainnet 1..200 (`swiftsync-verify/vectors/mainnet_1_200_engine.hze`, its image id and key) and its
  block-200 trie snapshot (`zapps/statement/vectors/engine_200.trie`, node format 2, root equal to the proof's). No key
  of ours was paid in blocks 1..200, so the positive case on the real trie is a zero balance for fresh keys; non-zero
  cases (P2WPKH, P2TR, P2SH-P2WPKH, P2PKH full and legacy, P2PK compressed and uncompressed) use a trie built in the
  test under the real proof's state with its trie root replaced. Refusals: tampered total and fact answer; wrong height,
  block, core image and network; core_id and block_hash in upper case, empty, or carrying a line break; another purpose
  or nonce; a purpose rewritten to match; another key's signature; a signature over another message; a fact relabelled
  to a script it does not prove; a duplicate or unsorted script; more than 10,000 entries; a history fact; a fact with
  no answer; a purpose or nonce with a line break, U+2028, U+2029, U+202E, U+200B, a tab, a non-ASCII letter, or
  surrounding spaces; a wrong format; no entries; another version; a time-locked proof; a 65-byte P2TR signature ending
  0x00; the recency check; and the P2PK cases of §7.
- `zapps/common`: 5 tests, among them sources that disagree giving no answer (`resolve`, `expect_at`), a node that
  gives no chainwork failing, and the network tip height for `--max-age`.
- `zapps/statement/test_statement.sh`: `zapp-statement` (checker and producer) end to end on the engine proof with
  fresh P2WPKH, P2TR and P2PK (both forms) keys, with the chain-identity sources, `--max-age` 6 and 7 blocks behind a
  node's tip and with no tip source, a node without chainwork, a purpose with U+2028 or a trailing space; with
  `STATEMENT_WASM=<zapp-statement.wasm>` it also runs `wasm/test-statement.mjs` (18 cases) through the zAPP's WASM
  module: 39/39 (`test_statement.out`, 27.11.2026). The best verdict is DEMO: 200 blocks are below Core's minimum work.
- The `bip322` crate's own suite (77 tests, including BIP-322's official vectors) passes at 0.0.12.

## 10. Open

- Per-coin statements (BIP-322 "proof of funds", which signs the coins' outpoints) instead of per-script: stronger
  (the coins themselves are signed for) but needs a coin fact with its creating transaction for every coin.
- A review of `p2pk.rs` before the module is released, and P2PK vectors of our own published with it.
- A version 2 message with a `holder:` line (§4, relay), if checkers want the claimed holder as a field of its own.
