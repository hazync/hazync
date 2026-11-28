# Round 3, review 4: the SwiftSync hints zAPP and the Utreexo roots zAPP (base and tip mode)

Reviewer: independent, read-only. Tree: `r3tree` at `a9b78510`. Date 28.11.2026.

Scope: `prover/zyncapps/methods/{hints,utreexo,utreexo-tip}` (guests), `libzync/src/hints.rs`,
`libzync/src/utreexo_roots.rs`, `prover/zyncapps/methods/utreexo-tip/src/step.rs`, `zapps/hints` (lib, CLI, wasm, ffi),
`zapps/utreexo` (lib, tip.rs, CLI, wasm), `docs/libzync/HINTS_ZAPP.md`, `docs/libzync/UTREEXO_ZAPP.md`. Also read, because
the tip guest runs it unchanged: rustreexo 0.7.0 (`b73cbac1`, from the local cargo registry), `Stump::modify` and
`Proof::calculate_hashes_delete`. Also read: the engine `env::verify` (engine/platform/risc0-zkvm/src/lib.rs:156-178) and
`verify_full_bytes` (engine/verify-core/src/lib.rs:183-188), only to confirm how the zAPP binds its children.

## Question and answer

Can a prover get a reader to say OK for a hints file that is not the true unspent-at-H list, or for Utreexo roots that
are not Floresta's accumulator after block H (including through tip mode, the witness, or the starting roots)?

**I found no way to do it, as long as the reader is given the right image ids and keys.** Every input from the host is
either checked against a hash the core proof commits, or opened against an accumulator the guest holds itself. The
findings below are about what the readers pin and how they present the result. None of them is a forgery.

The weakest point is outside the guests: today the readers have **no built-in ids**. The hints image, the base image, the
tip image, the core image and the key pin all come from the caller (M1). OK means "correct, if those ids are the
ones you meant".

## Findings, by severity

### M1 (Medium, deployment): OK rests entirely on ids the caller types in, and the hints CLI does not say so

- `zapps/hints/src/lib.rs:16-18,70-77,106-110`: the hints image id, its key and pin, and the core image id are all
  arguments. There is no built-in set to compare them against.
- `zapps/utreexo/src/lib.rs:21,26,40-46`, `zapps/utreexo/src/tip.rs:20-22,63-69`: `UTREEXO_IMAGE_ID` and
  `UTREEXO_TIP_IMAGE_ID` are `""`, so every id is `IMAGE_CALLER`. A caller id **does not cap the verdict**
  (`lib.rs:248`, `tip.rs:183-184`: only `IMAGE_BUILTIN_UNPINNED` caps it at DEMO).
- Scenario: a server publishes `proof.hze` together with a "how to verify" line that gives `--image X --vk k.bin
  --vk-sha256 sha(k.bin) --core-image C`. X is an image the attacker wrote, which commits any `HintsStatement` or
  `UtreexoStatement` it likes, with `core_id = C` and a profile, anchor and work that pass. C can be the attacker's
  "core" too. Both readers return OK (exit 0). Nothing in the code is wrong here. The trust decision has simply moved
  to whoever supplies the ids.
- The utreexo CLI warns about this on its OK line (`cli/src/main.rs:209-211`: "under the utreexo image you named; this
  checker does not know its code"). **The hints CLI's OK line has no such warning** (`zapps/hints/cli/src/main.rs:107-109`),
  and neither the hints wasm JSON nor the FFI message reports where the image came from. That is a parity gap.
- Fix: until the Stage 4 freeze, cap caller-named ids at DEMO unless an explicit `--trust-image` flag is given. Or at
  least print the utreexo caveat in the hints CLI, wasm (`image_source`) and FFI too. At the freeze, pin the hints,
  base, tip and core ids per network, as the core verifier's table does.
- Confidence: high (a direct read of the code).

### L1 (Low): the hints reader takes the network from the proof; the utreexo reader takes it from the caller

- `zapps/hints/src/lib.rs:105`: `net = if z.profile & PROFILE_SIGNET != 0 { "signet" } else { "mainnet" }`. The CLI then
  asks `--mempool` / `--bitcoin-cli` about **the proof's** network (`zapps/hints/cli/src/main.rs:85-87` →
  `zapps/common/src/lib.rs:30-36`, which picks mempool.space/signet). The OK line does not name the network
  (`cli/src/main.rs:107`).
- Utreexo does the opposite on purpose (`zapps/utreexo/src/lib.rs:137-153,221-224`; `cli/src/main.rs:13-14,128`: "the
  proof never picks the network").
- Mitigation that holds today: the core image is built per network (`prover/methods/guest/src/main.rs:78-85`, the
  network is a compile-time feature), so a signet proof fails `z.core_id == --core-image` unless the user names the
  signet core id. With `--mempool` on the default base, a user who names the signet core id by mistake gets an
  OK for signet, on a line that never says signet.
- Fix: add `--network` (default mainnet) to the hints lib, CLI, wasm and FFI, as utreexo has, and put the network in
  the OK line.
- Confidence: high.

### L2 (Low): no expected-format input; a valid format-2 file passes a format-1 node

- Formats 1 and 2 are the same indices in different bit orders (`libzync/src/hints.rs:18-22,106,172`). The reader
  accepts either (`zapps/hints/src/lib.rs:89`) and decodes with the format the proof states. `verify_hints`, the wasm
  frame and `swiftsync_verify_hints` (ffi/src/lib.rs:41-45) take no "format I read" argument. The FFI returns `format`
  in `out` (ffi/src/lib.rs:74), but the header does not say a caller must check it.
- Scenario: a server serves a genuine format-2 proof and file to a node patch that reads hintsfile.md bit order. The
  verdict is OK, the node misreads the indices, and the SwiftSync aggregate fails at the end of the sync. That is the
  wasted sync the zAPP exists to prevent. It is not a soundness break: the statement is true for format 2.
- Fix: an `expect_format` argument, or a FAIL unless `format` equals the one the caller names.
- Confidence: high.

### L3 (Low, informational): the undo file is optional, so `undo_sha256` is often unchecked

`zapps/hints/src/lib.rs:99-102`: the undo-hash check runs only when a file is given, and an OK without it says nothing
about undo data. The C header documents `undo` as optional, which is fine, but the OK message
(ffi/src/lib.rs:77) does not say whether undo was checked. Suggest adding "(undo not checked)" to the OK message.

### I1 (Info): tip mode trusts `Stump::modify` alone, where Floresta also calls `verify`

`step.rs:321-324` calls only `s.modify(adds, dels, proof)`. Floresta calls `Proof::verify` before `modify`. I checked
whether `modify` alone accepts a proof that changes the result:
- rustreexo `calculate_hashes_delete` refuses `targets.len() != del_hashes.len()` (proof/mod.rs:529-534) and a missing
  or misplaced sibling (:576-582). `modify` refuses unless every computed old root equals a current root and all are
  consumed (stump/mod.rs:317-334).
- The del hashes are fixed by the guest: each comes from a bound block input plus host coin data
  (step.rs:299-307). They are distinct (outpoints de-duplicated at step.rs:266; the leaf commits the outpoint), and
  they are leaf hashes (an SHA-512/256 input of at least 81 bytes after the 128-byte tag block). So they can
  never equal a 64-byte parent preimage.
- So an accepted proof's raw old tree must be the real preimage of that root (collision resistance), and the new value
  is the identity fold of that same tree with the del leaves set to Empty. Roots are replaced by value
  (stump/mod.rs:320-327), so even a proof that names the "wrong row" for a collapsed tree (a root whose identity-fold
  value is shallower than its row) yields the same new value. Duplicate targets need two distinct del leaves to be
  real siblings, and then they give the honest result.
- My conclusion is that `modify` alone is sound for this use. Confidence: medium-high. I reasoned about this and
  did not fuzz it. A differential test (an honest proof against mutated proofs with duplicate, odd-position and
  wrong-row targets, all through `modify`) would settle it cheaply.
- Separately, `calculate_hashes_delete` computes the OLD parent as a raw `parent_hash` even when a sibling is Empty
  (proof/mod.rs:592). A deletion next to a fully deleted non-root subtree would then be refused. That is a liveness
  question shared with Floresta, not a soundness one.

### I2 (Info): dead line and wording

- `zapps/hints/src/lib.rs:113` (`lineage == 0 || lineage_of(img.lineage)`) can never decide anything, because
  `:103` already requires a zero lineage. Utreexo removed the same dead line (lib.rs:216-219). Remove it here too.
- The utreexo tip reader accepts the image's handover lineage for a step (tip.rs:156), which is correct (a step needs
  no undo). The base's zero-lineage rule is enforced inside the base guest (utreexo/src/main.rs:202), and the tip
  reader pins `base_core_id` (tip.rs:153), so the base's trust carries. That checks out.

## What I checked and found sound (evidence)

**Binding to the core proof (all three guests).** `env::verify(core_id, journal)` is an OpenVM deferral whose
SHA-256(app_exe_commit ‖ app_vm_commit) must equal `core_id` and whose public values must equal SHA-256(journal)
(engine/platform/risc0-zkvm/src/lib.rs:165-176). `check_core_statement` (libzync/src/facts.rs:378-404) checks:
KIND_CHAIN, version ≥ 2, `self_id == core_id`, the network bits, no assumevalid, the coin-log profile, and the genesis
anchor compiled into each guest (hints main.rs:90-91,137; utreexo main.rs:101-102,201; tip main.rs:120-121,234;
`zapps/hints/src/lib.rs:146-154` pins the hints guest's anchors to the core table). The hints and base guests also
require a zero lineage and a core image not in `PRE_UNDO_IMAGES` (hints main.rs:139-140; utreexo main.rs:202-203), and
H ≤ T. The readers re-check `core_id`, the whole `profile`, `anchor_id`, `max_time`, min work and checkpoint, block H
against the node's own chain, and the tip when T > H (hints lib.rs:103-130; utreexo lib.rs:211-248; tip.rs:141-184).

**The blocks are the chain's (hints, base).** Each header must follow the previous one (hints.rs:427,
utreexo_roots.rs:438). Transactions are parsed strictly without witness (a marker gives nin = 0 and is refused),
there are no trailing bytes, the Merkle root must match with Core's mutation check, tx 0 must be the only coinbase (so
the 64-byte-tx depth trick needs a coinbase-shaped inner node), and compact sizes are read as Core reads them
(hints.rs:283-338,432-456; utreexo_roots.rs:223-278,443-467). The block forest 0..T is rebuilt with 0x05 leaves over
(hash, height, state, undo) and must equal the journal's `block_roots`/`block_leaves` (hints main.rs:173-174; utreexo
main.rs:219-220). That binds every header and height, every `state_h` and every `undo_h`. Tail leaves above H are
counted exactly (`n == T − H`) and folded in the same forest, so a substituted tail leaf breaks the roots.

**Spent/unspent at H (hints, base).** Outputs are walked in block order. Unspendable outputs (OP_RETURN first, or over
10,000 bytes, as Core's `IsUnspendable`) and the two mainnet BIP30 coinbases (matched by height and hash) are skipped
by rule. Same-block spends are found from the block's own inputs and are never asked of the host (hints.rs:457-525;
utreexo_roots.rs:468-532). The host gives a status only for outputs that enter the coin log. Each one is rebuilt as
`coin_leaf` or `spent(coin, spender(w,tx,vin))` with `h < w ≤ H` (hints.rs:517; utreexo_roots.rs:522). At H the
walked coin state must equal `state` (hints.rs:542-551; utreexo_roots.rs:544-553), and that state is inside block H's
leaf, which the forest equality binds. A flipped status gives a leaf the coin log does not have. Indices,
same-block-spent counting and the BIP30 exclusions are deterministic functions of the bound block, so the host cannot
skip, duplicate or reorder outputs. The canonical Elias-Fano encoding is re-checked on decode (hints.rs:196-199), and
the file header's H is inside the hashed bytes.

**Base runs (utreexo).** A continuation verifies the previous run under the committed `self_id`, with `done == 0`, the
same network and H, and the state bytes' SHA-256 equal to `state_out`. The decoder is canonical (re-encode equals
input, utreexo_roots.rs:692) and the state ends where the statement says (utreexo main.rs:153-166). The reader requires
`self_id` to equal the verified image (lib.rs:214), so every run is that image. The whole walk state (both forests,
MTP window, last hash, BIP30 flag, Utreexo forest) is carried, so the final run's forest checks cover every block of
every run.

**Floresta compatibility (base).** Leaf = SHA-512/256(TAG ‖ TAG ‖ block hash ‖ outpoint ‖ header code ‖ TxOut)
(utreexo_roots.rs:118-132). Adds skip unspendable and same-block-spent outputs; the BIP30 coinbases are added live and
never deleted (:504-511). The roots are the identity fold with deleted leaves Empty, which matches rustreexo's add rule
(stump/mod.rs:530-535) and its delete rule (proof/mod.rs:584-589). The doc's cross-checks against floresta-chain via
`prover/zyncapps/utreexo-ref` are mainnet 1..100,000, Floresta's published 169/175 values and signet 1..120. I did not
re-run them.

**Tip mode.** The previous receipt is a final base run of `base_id` (`done == 1`, `to == height`) or a step of
`self_id` with the same `base_id`, on the same network (tip main.rs:163-190). Each block follows the previous one from
the prior statement's `block_hash`; the T − H link headers must reach the core tip (main.rs:236-249). So, by second
preimage resistance, every block of every step is an ancestor of the pinned tip, and the heights are fixed by the
counts. That is why earlier steps' core receipts do not matter (UTREEXO_ZAPP.md §10.4, which I agree with). Adds and the
deletion set come from the bound block (step.rs:233-273). Each deleted leaf is rebuilt from the input's outpoint and
must open (I1). Deletions of Floresta's two BIP30 leaves are refused, and the constants are tested against the real
coinbases (`zapps/utreexo/tip-step/src/lib.rs:79-101`). Starting roots: `stump()` requires one root per set bit, and
zeros map to Empty both ways (step.rs:41-62). The reader pins `self_id`, `base_id`, `base_core_id` and `core_id`
(tip.rs:143-154). The witness source (forest or native) is never trusted: a wrong coin record or proof makes the guest
panic.

**Wasm/CLI/FFI parity.** All of them call the same `verify_hints` / `verify_utreexo_on` / `verify_utreexo_tip_on`. The
key pin is enforced in `verify_full_bytes` (verify-core lib.rs:184) and so on every path. The FFI also checks it first
(ffi/src/lib.rs:63-68). Wasm frames refuse trailing bytes and overflowing lengths. The utreexo wasm and CLI pick base
or tip by the unverified journal kind, but each path then verifies the proof and checks the kind
(lib.rs:211, tip.rs:141). Differences: hints' network handling (L1) and the missing caller-image caveat (M1).

## Not verified

- The OpenVM deferral itself (`verify_stark_unchecked`, the exe/vm commits): reviews 1 and 8. I took
  `env::verify` as binding (image id, public values).
- That the engine core guest's coin log matches the zAPP walk at modern heights: segwit-era blocks, the
  `tracked` bit and the creation-MTP rule. A mismatch would only stop proofs (liveness), but I did not run anything.
  The doc says modern blocks are unmeasured.
- Floresta at its current head: I read rustreexo 0.7.0 from the registry, not Floresta's source. Its
  `get_block_adds` / `process_proof` / `is_unspendable` are taken from the doc's citations. I did not re-run
  `utreexo-ref` or any test (no builds, to keep disk use down).
- `engine/guest/coinlog_ovm.py` and the engine assembly of these guests (`engine/zapps/assemble_guest.sh`): read
  only enough to see that the utreexo guests use software SHA-2. Their compress path was not compared with the core's.
- I1's soundness argument for `modify` alone was reasoned, not fuzzed.
