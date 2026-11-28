# Round 3, review 5: assumevalid / assumeutxo checker and the deposit zAPP

Tree: `r3tree` at a9b78510. Read-only review; nothing was built or run (the OpenVM verifier build is heavy, so no test
was executed here). File:line references are to that tree.

## Verdict

**I found no way to get OK, or a misleading DEMO, for a false statement** in the assume checker, the deposit zAPP or
corecheck, provided the image id and both pinned keys are what the caller thinks they are. Every candidate attack
below is closed, and each closing check is cited. What remains is design notes, doc drift and small parity or DoS
nits, ranked below. None is above LOW.

## Findings, ranked

### L1. With `require_identity` off, minimum chain work does not stop a fork above the min-work height (LOW, by design but under-documented). Confidence: high
`zapps/deposit/src/lib.rs:403-411`: `enough = headers_ok` (or `core_ok`), and Ok needs only `enough` when
`!ask.require_identity`. The min-work gate (`zapps/headers/src/lib.rs` `work_at_least(&s.work, mainnet.min_work_hex)`;
core `swiftsync-verify/src/lib.rs` `statement_checks`) compares cumulative work. A fork taken off the real chain
*after* the height where the chain passes `nMinimumChainWork` (Core v32.1's value, around the 966k assumevalid) inherits
that work.

Scenario: start from the real headers 0..F, F past the min-work height and past 970,000. Mine `min_depth` blocks at
the current difficulty, one holding the attacker's "deposit". Prove the core chain through the fork (it is a valid
chain) and prove the header chain on top. The result is `accepted = true` with `--no-require-identity`.

The cost is exactly `min_depth` real blocks: the usual SPV confirmation economics. DEPOSIT_ZAPP.md §1 (lines 34-35)
explains DEMO as "a difficulty-1 fork from genesis", which can read as if min-work alone made OK forge-proof. It does
not. Identity (on by default) or the caller's own tip comparison is what rules this fork out.

Suggest one sentence in §1 next to `--no-require-identity`: without identity, OK means "min_depth blocks of real work
were spent", no more. No code change is needed.

### L2. ASSUME_ZAPPS.md trust and binding sections still describe the RISC Zero chain (LOW, doc). Confidence: high
- `docs/libzync/ASSUME_ZAPPS.md:186` lists "RISC Zero's soundness" as trusted. It should be OpenVM v2.1.0 plus the
  pinned keys, as DEPOSIT_ZAPP.md §4 says.
- `:81` says "every published image: `0x71` mainnet, `0x75` signet". Engine images are `0xf1`/`0xf5` (NODE2 bit 7,
  `swiftsync-verify/src/lib.rs:28,44`), and no image is published yet.
- `:190-197` (how `live_hash` is bound to the coin log) cites RISC Zero guest file lines (`agg_lite.rs` 688/709/719/736,
  `main.rs` 2294/2335). The engine guest is built from that source through `engine/guest/edits.py` and patches, so those
  line numbers may not describe the engine image. This is the one load-bearing soundness argument for assumeutxo.
- `:186` "rebuild it: `reproduce/`" is RISC Zero tooling; the engine's reproducible build is the Stage 4 freeze.

The engine-era ECMH map itself agrees with the checker: `zapps/assume/tests/engine.rs` matches the real 0xf1 proof at
200.

### L3. C ABI vs WASM: an empty buffer means "absent" in WASM but is a FAIL in the C ABI (LOW, parity; fails closed). Confidence: high
- WASM (`zapps/assume/wasm/src/lib.rs:108-110`): `headers`, `snapshot` and `tracked` of length 0 mean "not given".
- C ABI (`zapps/assume/ffi/src/lib.rs:102-103`): a non-NULL pointer with length 0 is `Some(&[])`. The headers then fail
  `bind` ("0 bytes is not a positive multiple of 80") and the snapshot fails "truncated".
- Also `:97`: the "neither assumevalid nor snapshot" test looks at `snapshot.is_null()`, not at its length.

The C ABI is stricter, so this never produces a false OK. A C caller that passes `(buf, 0)` for "none" gets FAIL. The
header (`ffi/include/zapp_assume.h:38-41`) should say NULL, not a zero length, means none.

The other differences are documented: bundles (.sspf) are refused by the C ABI and WASM and accepted by the CLI; the
CLI clamps `--now` and WASM/FFI trust it.

### L4. Unbounded leaks in corecheck's caches (LOW, DoS for long-lived callers). Confidence: high
`zapps/corecheck/src/lib.rs:69-77` leaks one `Image` per distinct `(network, image_hex, ..)`. `:109-113` leaks one
`Image` holding a full copy of the vk (about 96 KB, `swiftsync-verify/vectors/mainnet_1_200_engine.vk`) per distinct
key.

The caller supplies both the key and its pin. So a server, or a long-lived page, that verifies attacker-supplied
(image, key, pin) triples grows by about 96 KB per request, with no bound. The comment at `:65-66` addresses repeated
identical requests only. Suggest a size cap, or documenting that a server must pin the (image, key) set itself.

### L5. Deposit says "in block", not "spendable" (INFO). Confidence: high
`verify_deposit` accepts any position, including index 0 (a coinbase). That covers immature coinbase outputs (fewer
than 100 confirmations), the genesis coinbase (a leaf at height 0 opens; its output was never in the UTXO set), and the
two BIP30-duplicated coinbases (91,722/91,812, the same txids as 91,880/91,842). Each statement is true (the
transaction is in that block), but the output may never be spendable.

DEPOSIT_ZAPP.md §3 already says "still unspent: not shown" and points to the liveness zAPP. Coinbase maturity is not
mentioned. A bridge should require `vout`/`script` it controls (it normally does), and either refuse `index == 0` or
require depth >= 100 for a coinbase. Worth one line in §3.

### L6. Stale fuzz artifacts (INFO). Confidence: medium
`zapps/assume/fuzz/artifacts/assume_core_receipt/crash-poseidon2-unreduced-word-at-225087.rcpt` and `leak-1ef0…` are
RISC Zero `.rcpt` inputs. `fuzz/tests/replay.rs:1-3` says artifacts "move to the corpus only once their fix has
landed". The engine verifier refuses any non-`HZE1` file at once (`swiftsync-verify/src/lib.rs` `verify`), so these no
longer reach the code they crashed. Either retire them, or record that they are obsolete, so they do not read as an
open crash.

## Attacks checked and closed (with evidence)

**assumeutxo: a snapshot whose coins differ from the proven set**
- Every snapshot field is in the leaf preimage: txid, vout, value, script length and script, height, coinbase bit,
  tracked bit, creation MTP (`zapps/assume/src/lib.rs:237-249`). Equal leaf multisets therefore give equal coin sets,
  field for field.
- The side inputs cannot help a false set:
  - The headers are bound by hash linkage to `tip_hash` (`:270-295`) and must start at genesis (`:840`), so every MTP
    is fixed by the tip.
  - The tracked bit only changes leaves. Choosing it adversarially is an ECMH collision with chosen elements.
- Multiset and encoding ambiguities:
  - Duplicates and split groups are impossible: txids strictly ascending (`:584-590`), encoded vouts strictly ascending
    (`:601-605`), the count checked exactly, trailing bytes refused (`:592-593`, `:616-621`).
  - A coin cannot be "cancelled" by its negation: the map gives only even y (`:102`; guest
    `prover/methods/guest/src/swiftsync.rs:88-91`), so -P is never an element. The verifier side only adds.
  - Non-canonical encodings inside the file are accepted the way Core accepts them: a raw script that could have been
    compressed, or a wrapping DecompressAmount (`:492-511`; MoneyRange checked after, `:608`). The decoded coin is what
    enters both the leaf and `hash_serialized_3`, so the file may differ from Core's bytes but the coin set cannot.
  - Type 4/5 keys off the curve are refused (`:532-536`); scripts over 10,000 bytes are refused (`:539-543`).
- The printed `hash_serialized_3` (`:630-647`) matches Core's TxOutSer (outpoint, `height<<1|cb` u32, CTxOut, SHA-256d)
  in Core's key order, which `:577-605` enforces. After a pass it is a function of the proven set.
- Base block = `tip_hash`, network magic = the proof's (`:846-853`); total value = `utxo_value` (`:856`); live hash
  equal (`:859`).
- ECMH arithmetic:
  - Jacobian doubling (`:125-141`, a=0) and mixed add (`:144-174`) are standard formulas.
  - P + (-P) gives the identity (`:157-163`). No y=0 point exists (prime order).
  - `inverse().expect` is reached only when z != 0 (`:180`, `:200`).
  - Tested against a num-bigint reference (`:987-1047`).
  - The parallel `PointSum` joins every worker and surfaces any error (`:802-821`).
- Heights: `code>>1` fits in u32; height 0 is refused, as is a height above the base or the headers' tip (`:699-705`);
  the MTP cache is bounded by the headers' tip.

**assumevalid: a wrong block**
- `assume_valid` (`:366-388`) refuses a non-libzync profile and the scripts-skipped bit 1. It accepts only the tip, or
  a block inside headers that hash-link to the tip and end exactly at `cs.height` (`:380-383`). Heights come from the
  distance to the tip.
- A journal can only carry bit 1 if the image's own profile has it, because the core verifier requires
  `cs.profile == image.profile` (`swiftsync-verify/src/lib.rs` statement_checks, step 9).

**Proofs from other networks or images**
- `image_for` is scoped by network (`zapps/corecheck/src/lib.rs:80-96`).
- The core verifier checks the profile (which carries the signet bit) against the image, and `anchor_id` against the
  network's genesis anchor.
- The engine front recomputes the image id from the pinned key (`engine/verify-core/src/lib.rs:165-171`), refuses a
  key whose SHA-256 is not the pin (`:183-186`), and binds the journal by SHA-256 to the public values (`:174-178`).
- The deposit refuses a bundle network that differs from the caller's or the image's (`zapps/deposit/src/lib.rs:214`),
  and header runs off mainnet (`:234`). A signet header MMR cannot contain a mainnet core tip, because of the binding.
- Every surface pins the key:
  - CLIs: `read_pinned_key` plus `with_key`.
  - WASM: the pin is in the frame, checked by `with_key` and, for headers, by `verify_full_bytes`.
  - C ABI: a required 32-byte pin.
  - No surface accepts an unpinned key.

**Deposit: a transaction not in that block, at another depth, or on another chain**
- Txid binding: the txid is the caller's, and the raw bytes must hash to it. In the full path the txid is recomputed by
  `tx_in_block` (`libzync/src/facts.rs:170-185`) and compared (`zapps/deposit/src/lib.rs:291`). In the PoW path it is
  checked by `verify_tx_in_header` (`zapps/headers/src/lib.rs:500-541`) and again by libzync's parser (`:333-339`). The
  outputs come from the same parse whose re-serialised stripped form is the txid preimage (`facts.rs:240-298`).
- 64-byte and CVE-2012-2459 family:
  - Raw length 64 is refused early (`:275`); a stripped length of 64 is refused in both paths (`facts.rs:171`;
    `headers/src/lib.rs:519`).
  - The coinbase must parse with one input and the null prevout, must not be 64 bytes stripped, and must climb to the
    root at position 0 with a branch of the same length (`facts.rs:171-182`; `headers/src/lib.rs:524-535`).
  - A deeper fake coinbase would need a txid matching the zero bytes of a real coinbase's prevout, which is
    infeasible.
  - `merkle_climb` refuses `pos >= 2^len` and an equal sibling on the right-child side, which is the duplicated-last
    position (`facts.rs:138-160`; `headers/src/lib.rs:488-498`).
- The block is in the validated chain: the forest leaf commits the hash, the height and the state, and its position is
  the height (`libzync/src/undo.rs:72-98`; leaf preimage `prover/methods/guest/src/handover_pin.rs:58-81`). The forest
  shape is checked against `block_leaves`, which the core verifier forces to equal `height + 1`.
- Binding the core tip to the header chain:
  - `verify_inclusion(&s, cs.height, &cs.tip_hash, path)` (`zapps/deposit/src/lib.rs:254`) uses the MMR's tagged
    leaf/node/bag hashes, checks the peak count against the leaf count, and checks the path length against the peak
    height (`prover/zyncapps/methods/headers/src/mmr.rs` `verify`).
  - Both chains are hash chains, so header blocks 0..Hc are the core's.
  - Using `headers_ok` instead of `core_ok` when headers are given is sound for that reason (subject to L1).
- Header-only blocks: a block above Hc needs `allow_pow_only_block` and `min_validated_depth` 0, and both openings
  cannot be given (`:280-313`). The statement carries `block_fully_validated: false`, and the CLI says "THE BLOCK ITSELF
  IS ABOVE…" (`cli/src/main.rs:281`).
- Depth: `confirmations = tip - h + 1` against the header tip (or the core tip), `validated = Hc - h + 1`
  (`:379-380`). Identity compares that same tip (`:392-395`). The CLI's `--mempool`/`--bitcoin-cli` path asks the
  sources for exactly that tip height and fails if they disagree (`zapps/common/src/lib.rs:161-179`).
- The tip time used by `max_tip_age` is the tip's own time (`prev_time` is "timestamp of the tip block",
  `prover/methods/guest/src/main.rs:1468`).

**Gating and verdict parity**
- assume, all three surfaces:
  - assume checks run only after a non-FAIL core verify (CLI `cli/src/main.rs:93-102`; WASM `:101-105`; C ABI
    `:85-88`).
  - Any failed assume check is a FAIL (CLI `:106,110`; WASM `:112`; C ABI `:106-108`).
  - OK requires the tip to have been compared (CLI `:112-113`; WASM `:113`; C ABI `:127-132`).
- Deposit: CLI and WASM call the same `verify_deposit`. The WASM ask refuses unknown keys and nulls
  (`wasm/src/lib.rs:56-63`). Its `min_depth` default of 1 equals the library's `max(1)`.
- Frame parsing: `Frame::take` is overflow-safe and the frames refuse trailing bytes (`assume/wasm/src/lib.rs:37-47,74`;
  `deposit/wasm/src/lib.rs:41-50,148`).
- `verify_core` downgrades a bundle that does not decode to FAIL with no state (`corecheck/src/lib.rs:129-140`). A tip
  that matched in a failed report is not `tip_checked` (`:159`).

## Not verified
- Nothing was executed (no cargo test, CLI or WASM run). Every behaviour above is from reading the code.
- That the **engine** guest binds `live_hash` to its coin log, and computes the coin leaf and the ECMH map, exactly as
  the RISC Zero guest source does (see L2). I read the RISC Zero guest's `swiftsync.rs`; I did not trace
  `engine/guest/edits.py` or the patches.
- The header-chain guest's PoW, retarget and genesis rules, and OpenVM's soundness: assumed.
- ECMH collision resistance with chosen elements (DL plus a random-oracle hash to the curve): assumed as the doc states.
  The 0/1 multiplicities (enforced by key order) leave a subset-sum in a prime-order group, for which no
  sub-exponential attack is known.
- `swiftsync-verify`'s `check_tx_fact`, `block_in_chain` and the coin-log internals beyond the lines cited.
- The `zapps/common` network fetch paths (curl/https, node timeouts) beyond the agreement logic.
