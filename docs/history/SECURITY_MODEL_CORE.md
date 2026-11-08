# Hazync security model

What a Hazync proof guarantees, what it rests on, who can attack what, the soundness argument and its
known gaps, what happens to proofs when the guest changes, what has been reviewed, and what has been
fuzzed. Written to pre-empt critique, not to hide it. Read the root [`../SECURITY.md`](../../SECURITY.md)
(audit status, finding index, vulnerability reporting) before relying on any "undeniable" framing.

**Scope.** Sections 1-6 describe the original system, the canonical CORE guest, as of September 2026
(threat model checked against code at `d5286b8`, v0.21.4 plus 13 commits, canonical guest `37987b85`, on
2026-09-14). libzync has its own statement and verifier: §7. Where a doc and the code disagree, the code
wins. Terms: [`OVERVIEW.md`](OVERVIEW_CORE.md). Routes and configuration: [`COORDINATOR.md`](../COORDINATOR.md).
History of findings: [`history/SECURITY_AUDIT_LOG.md`](SECURITY_AUDIT_LOG.md).
Report a vulnerability privately: [`../SECURITY.md`](../../SECURITY.md#reporting-a-vulnerability).

Merged from SOUNDNESS, THREAT_MODEL, EXTERNAL_REVIEW, METHOD_ID_DURABILITY, PROOF_DURABILITY and FUZZING.

---

## 1. What a proof guarantees, and what it assumes

### 1.1 The claim

A verified **chain-tip proof** attests: every block from the anchor to the tip is valid under Bitcoin
Core consensus, the UTXO set is exactly the committed root, and cumulative proof-of-work is as committed,
checkable by verifying one succinct proof without re-executing history or trusting peers.

Concretely, a receipt that passes SPEC §11.1 ([`SPEC.md`](SPEC_CORE.md)) — verifies against the canonical
`METHOD_ID`, `journal.self_id` equals it, `journal.kind == KIND_RANGE`, and the genesis pin of SPEC §9
holds — attests that blocks `1..hi` are valid and that the UTXO set is the committed out-boundary.
Liveness, attribution, availability and cost are outside that statement.

### 1.2 Trust base

1. **Real Bitcoin Core v28.0 code.** The script interpreter, sighash, `CheckTransaction`, the
   transaction/weight/sigop machinery, `ComputeMerkleRoot`, the difficulty retarget (`pow.cpp`'s
   `CalculateNextWorkRequired` / `CheckProofOfWorkImpl`, driven through the real `CBlockIndex` /
   `chain.cpp`, constants from `kernel/chainparams.cpp` and `primitives/block.cpp`) and libsecp256k1
   v0.5.1 are the actual Core sources compiled into the zkVM, not a reimplementation. This removes the
   reimplementation-soundness gap every prior effort carries. TU list: `prover/methods/guest/build.rs`.
   The shims are portability-only, plus libc/unwinder glue: `patches/0001-serialize-ilp32-int-overload`
   (32-bit int overload), `patches/0002-sha256-route-through-risc0-accelerator` (byte-identical; a
   substitution, and the precedent for 0012/0013 — `history/CORE_VS_GHOST.md` §1), and single-thread
   no-op `coreshim/{sync,threadsafety,logging}.h`. **No consensus logic is changed.**
2. **Non-Core code beneath libsecp256k1 (since v0.21.0).** `provision-vps.sh` phase 5a applies two more
   patches unconditionally (`HAZYNC_FIELD_BIGINT2=1`, `HAZYNC_LIFTX_HINT=1`). Every signature check runs
   through them, so they are consensus-relevant, and they are ours:
   - **`0012` — the `field_bigint2` backend** (`prover/methods/guest/field_bigint2.h`,
     `field_bigint2_impl.h`, `src/field_bigint2.rs`). A narrow substitution at the field interface libsecp
     already parameterises: lazy 8×32-bit limbs, add/sub/negate/half in C, multiply/square/inversion via
     the RISC0 bigint2 coprocessor. wNAF, GLV, ECDSA/Schnorr logic and every check above the field stay
     libsecp's. The representation admits states (elements ≥ p) libsecp's own suite never generates.
     Gates 0-3 passed: a mod-p harness against arbitrary precision, libsecp's suite under `-DVERIFY`,
     mutation controls (two deliberately broken lazy backends pass libsecp's suite and are caught only by
     the harness), and a block-962,000 journal digest byte-identical to stock
     (`scripts/field-backend-tests.sh`, [`history/FIELD_BIGINT2_BACKEND.md`](FIELD_BIGINT2_BACKEND.md) §5b).
     ⚠ **Gate 4, the corrupt-signature negative control, has not run on a CORE build** — an open soundness
     item for shipped code (§3.6).
   - **`0013` — the `lift_x` witness hint** (`src/liftx_hint.rs`). `hazync_lift_x_hint` offers a host
     `y` for `secp256k1_ge_set_xo_var`, accepted only if `secp256k1_fe_sqr(y)` equals `x³ + 7`
     (`secp256k1_fe_equal`); the existing parity fix-up then fixes y uniquely, and a missing or wrong hint
     falls back to libsecp's `secp256k1_fe_sqrt`. Advice-and-verify: a hint can skip work, not change a
     result ([`history/LIFTX_HINT.md`](LIFTX_HINT.md) §3) — but that check is itself computed on the `0012` backend.
3. **SHA-256 collision resistance.** The Utreexo accumulator, the coinbase SMT and the merkle/commitment
   checks are SHA-256; that is their entire security. `sha2` comes from `risc0/RustCrypto-hashes` tag
   `sha2-v0.10.8-risczero.0` (`[patch.crates-io]` in the guest `Cargo.toml`).
4. **Project accumulators** — commitments *above* consensus, not Core reimplementations, but our code:
   the proven Utreexo `prover/methods/guest/src/utreexo.rs` (SEC-2 hardened, differentially fuzzed
   against `accumulator/src/lib.rs`), and the BIP30 coinbase SMT `coinbase-smt/src/{roots,bip30}.rs`,
   `#[path]`-included into the guest and CI-checked against a naive oracle sharing no code. With `0012`,
   these are the non-Core code most likely to hold any remaining soundness bug — the accumulator is "the
   single most likely location" (`../SECURITY.md`) — and none of it has been externally audited.
5. **RISC0 zkVM soundness** — STARK, recursion and, for the wrapped form, Groth16 including its trusted
   setup (SPEC §12). Pinned `=3.0.5`. A standard cryptographic assumption; breaking it forges anything
   and is out of scope.
6. **The anchor.** The chain proof starts from genesis or a trusted block-hash checkpoint. Everything
   after is proven; the anchor is a trust input, chosen by the deployment's trust ladder.

### 1.3 Scope of each proof type

- **Single-block / segmented** (`prove-full`, `prove-seg`): the block is *internally valid* (scripts,
  structure, no-inflation, PoW, retarget, merkle, subsidy, weight, sigops, witness commitment) and a
  *self-consistent* accumulator moves `root_prev → root_next`. It does **not** attest `root_prev` is the
  real mainnet UTXO set: the standalone harness builds `root_prev` from the block's own prevouts + filler.
  The 100000/130000/140000 runs are internal-validity demonstrations, never UTXO-membership proofs.
- **Chain proof** (recursion / range-fold): each step enforces `root_next(N-1) == root_prev(N)` back to the
  anchor, so the tip's UTXO root is the real set *given the anchor*. Real soundness for "these coins
  existed" lives in the chain. Since 2026-07-23 the archive-node bridge drives one resident Utreexo forest
  over the real chain and emits real `root_prev` + inclusion proofs, coin metadata and MTP; its receipt is
  byte-identical to the replay path, so the trust base is unchanged.

### 1.4 Not attested

- The **2-hour future-time limit** (G4): verifier-local wall clock, unprovable; deliberately unenforced.
- That the proven chain is the **most-work** chain. A proof commits `range_work`; the coordinator's
  `_frontier_chain` picks most-work, but the verifier CLI compares nothing against the network, and a
  proof for a competing chain is an open adversarial case for ghostd (`history/ROADMAP.md`).
- Anything below `lo` for a mid-chain range, or above `hi`; a transaction count (SPEC §14); any network
  but mainnet.

### 1.5 Verifier obligations

A verifier must assert the final `self_id == METHOD_ID` after `receipt.verify(METHOD_ID)` and pin the
leftmost in-boundary to the genesis anchor (full boundary, §3.2 H6). A chain receipt must be checked with
`verify-chain` (or the anchor pinned equivalently); a checkpoint-anchored proof correctly fails it. A
verifier that skips these has not checked the chain. The shipped verifier does all of this (§2.7).

---

## 2. Threat model

| component | inside the soundness trust base? | worst case if hostile |
|---|---|---|
| guest program and its inputs | **yes** | an invalid chain proves valid |
| risc0 zkVM, recursion, Groth16 | **yes** | a forged receipt |
| verifier CLI and C ABI | **yes** — it is the check | accepts what it should not |
| browser verifier, as served | **yes, for whoever loads it** | reports "verified" for anything |
| release signing, reproducible build | **yes, for anyone who does not rebuild** | a wrong verifier or host is trusted |
| ghostd adoption | **yes, for that node** | the node loads an unproven UTXO set |
| host / prover | no | wasted work; no false proof |
| GPU workers, rented hosts | no | latency, cost, a stolen contributor key |
| coordinator | no for proofs; yes for attribution and board figures | censorship, misattribution, stalls, false figures |
| worker CLI | no | local memory exhaustion, a stolen key, running a planted prover |
| websites | no, except the verifier module they serve | misinformation |

### 2.1 Guest program

`prover/methods/guest`, modes `chain_step`, `prove_range`, `fold_range`, `aggregate`; validation in
`validate_block` (`src/main.rs`). Trusted for SPEC §7 and the enforced list in §3.4.

**An attacker running the prover** chooses every witness byte. The guest stops them lying about height
(H1; H9 binds height into `boundary_digest`), swapping a spend inside a chunk (H2), double- or pre-spending
an in-block coin (H3), skipping `CheckTransaction` on the coinbase (H4), feeding phantom fee prevouts (H5),
fabricating the genesis in-boundary (H6), laundering another journal kind (H8), recursing against another
guest (S1), or controlling `has_witness` (SEC-1). What they can do is find a reject-valid bug and make a
valid block unprovable — liveness, not soundness; the audit record has several (G3, H-S2, #4, #8).

**An attacker who changes guest source** changes `METHOD_ID` — any edit that moves a line does, comments
and `#[path]`-included crates included (`reproduce/METHOD_ID`, `scripts/check-guest-inputs.sh`). Such
proofs do not verify against the canonical id, so the attack must be a public re-baseline: a new row in
`reproduce/LINEAGE.tsv` (derived from git, gated by `scripts/lineage.sh --check`) and an id anyone can
rebuild (`reproduce/Dockerfile`, CI job `reproducible-image-id`). That protects a verifier that pins the
id, and nothing that accepts whatever id it is handed — one reason there is no accepted set (§4.2).

### 2.2 risc0 and the vendored crates

`prover/Cargo.toml` patches two crates for the prover: `risc0-zkvm` → `vendor/risc0-zkvm` (upstream 3.0.5
except two files: recursion fold rebalanced into a tree, and assembly from externally proved segment
receipts; see `scripts/check-test-surfaces.sh`), and `risc0-circuit-rv32im-sys` → the #119 fix (zeroing a
LogUp cell before an add in two kernels, `history/releases/RELEASE_NOTES_v0.21.1.md`). #119 produced
receipts that failed their own `verify()` — fail-closed, a completeness bug. The host links the vendored
crates, so the coordinator's `verify-any` runs through them; `verifier/`, `verifier-ffi/` and
`verifier-wasm/` use crates.io `risc0-zkvm =3.0.5` unpatched. A vendored fault changes what gets proved,
not what the standalone verifiers accept.

### 2.3 Host / prover

Not trusted for soundness (SPEC §12: prover, witness and bridge are untrusted). Trusted **as a verifier**:
the coordinator runs `host verify-any` on every submission and `verify-range` then `verify-any` on every
spine; `/api/meta` reports `HAZYNC_HOST method-id`. A hostile proving host can prove a block against the
wrong predecessor (a fork, or a stale bundle after a reorg) at the right height; that receipt verifies but
cannot seam into the frontier, so `claim()` re-offers frontier+1 (#281, #284). It cannot make an invalid
block verify.

### 2.4 GPU workers, rented hosts, segments (all untrusted)

- **Segment workers** (`seg-serve`/`seg-connect`) cannot forge a segment receipt; a correct receipt for the
  wrong segment fails the join's state-digest continuity. Returned receipts pass
  `verify_integrity_with_context` before use ([`OPERATIONS.md`](../OPERATIONS.md), "Trust model"). Cost: latency.
- **Whoever controls a rented host** can read `$HAZYNC_HOME/key.hex` (mode 600 guards it from other users,
  not root) and submit, fold and beat claims as the contributor. Rotation (`rotate()`: old+new key
  signatures over `rotate_message(old, new, ts)` within `ROTATE_MAX_SKEW`) is something a thief can do, is
  recorded once per `old_pubkey` (the owner's later attempt gets `409`) and cannot be undone — so since #311
  `/api/rotate` answers `410` unless `ROTATE_ENABLED=1`; nobody had rotated on the public board. A stolen
  key can no longer move attribution, but can still submit and fold under the owner's name. Proofs are
  unaffected.
- **Sponsor bot pods** get a per-sponsorship key ([`COORDINATOR.md`](../COORDINATOR.md), "Identities"); SSH
  runs with `StrictHostKeyChecking=no` because pods reuse addresses. An on-path attacker could impersonate a
  pod: blast radius is spend within the bot's caps and attribution of sponsored blocks; every proof is
  still verified.

### 2.5 Coordinator

`coordinator/server.py`, public at `https://api.hazync.org/api/`. Trusted for liveness, allocation hints,
attribution, board figures, retaining receipts and serving bundles. **Not for validity:** every receipt is
downloadable (`/api/proof/<id>`, `/api/spine/proof`); the board's numbers are claims, the spine file is
evidence.

Enforced at its boundary:

- `verify_receipt` runs `host verify-any`, reads only the `RANGE-OK` stdout line, requires proven
  `[lo..hi]` to equal the claimed id. `VERIFY_MODE=mock` fails closed without `COORD_ALLOW_MOCK`; insecure
  modes refuse a non-loopback bind unless `COORD_ALLOW_PUBLIC_INSECURE`.
- `verify_sig`: ed25519 over receipt bytes; fails closed without the library unless `COORD_ALLOW_UNSIGNED`.
- `_frontier_chain`: most-work genesis-anchored chain, each seam requiring `in_tip == out_tip`,
  `in_bhash == out_bhash`, `lo == prev.hi + 1` (H7, S1/F1, H9); fuzzed (§6).
- `submit`: a multi-block range must be tiled by verified ranges (#281); a sponsor-held block only from its
  registered key (`_hold_refusal`).
- Inputs: `parse_any_range` shape-checks ids before any path; `clean_handle` strips `< > & " '`;
  `HANDLE_DENY`, `MOD_BLOCK_FILE`, `MAX_BODY`; per-IP limits with `X-Forwarded-For` honoured only from
  `TRUSTED_PROXIES`; `_verify_sem` bounds concurrent verification; static files contained to `COORD_WEB`.
- `sync_from_peers` re-verifies every peer receipt, shape-checks ids, bounds reads (64 MiB index,
  `MAX_BODY` receipt); a peer's reported handle is recorded as given.

**An outside attacker:**

- **Can claim blocks under any pubkey, unsigned** (#310). Signed claims (`claim:<nonce>:<ts>` within
  `BEAT_SKEW`) are verified; unsigned ones are accepted unless `CLAIM_REQUIRE_SIG=1`, because workers up to
  v0.21.4 sign nothing. `beat()`, `submit()`, `rotate()` are signed. Bounds: `CLAIM_OPEN_MAX` live claims
  per key (4, #319); no re-take of a block one's own unbeaten claim let lapse for `CLAIM_RETAKE_WAIT`
  (#321) — both count signed and unsigned apart, so unsigned claims cannot fill a key's signed slots but can
  crowd out its own unsigned (pre-signing-release) claims; unbeaten claims release after `CLAIM_GRACE` (600
  s, #296) and beats need the key; the frontier blocker is re-offered at most once per `CLAIM_TTL`. Nothing
  stops many fresh keys, and there is no per-address cap: public requests arrive from the web box, which is
  in `RATE_EXEMPT` (measured 2026-09-14), leaving nginx's `10r/s`. The effect on the board is not measured.
  Claims are advisory: `submit` accepts any height and `hazync run <n>` never claims.
- Can submit a valid proof against the wrong predecessor (§2.3), take any unreserved handle first, and read
  every pubkey (hence signed beats, audit #5 L-2).
- **Cannot** add an unverified range, advance the frontier with a non-seaming range, replace the spine with
  a shorter one (`submit_spine` compares under `_lock`), prove a sponsor-held block without its key, or
  rotate a key it does not hold.

**The operator, or anyone with the box,** can misattribute or hide contributors; edit, withhold or delete
receipts; refuse submissions; publish any figure; serve wrong or stale bundles (costing GPU time) or a stale
spine; report a different `method_id` in `/api/meta`, stopping every worker that compares ids (exit `78`);
and set sponsorship statuses by hand (BTCPay payments do not change that). It **cannot** make a receipt
verify for anyone else. Drift is visible: `/api/meta` publishes `source_sha256` of the running `server.py`,
compared by `scripts/check-deployment.sh`; on 2026-09-14 at 08:31 and 12:02 UTC it matched `d5286b8`.

### 2.6 Worker CLI

`coordinator/hazync` (released as `hazync-worker`). Trusted to hold the contributor key and sign only what
it produced: local receipt bytes, `<range>:<ts>` beats, `claim:<nonce>:<ts>` claims (#310), rotation
messages. It trusts the coordinator for which block to prove, bundles, fold inputs (`/api/foldable`,
`/api/proof/`), `/api/vranges` and the expected guest id.

A hostile coordinator can waste GPU time (bad bundles or fold inputs; `host fold-range` rejects bad
receipts, wrong proofs fail at submit); stop the worker with a different `method_id` (`EX_CONFIG`); and
**exhaust its memory** — `get()` returns `_open(...).read()` unbounded and `post()` parses the whole
response (open, §5.2 item 5). It cannot steer local paths: ids pass `parse_range` before becoming paths, and
`spine_index` follows only `/api/proof/` paths relative to `COORD_URL`.

**Prover discovery (#312, fixed).** With `HAZYNC_HOST` unset, `_find_host()` takes the first executable
named `hazync-host-x86_64-linux-gnu-cuda`, `hazync-host-cuda`, `hazync-host-x86_64-linux-gnu` or
`hazync-host` in the CLI's directory, then `$HAZYNC_HOME/bin`, then `$HAZYNC_HOME` (default `~/.hazync`);
bare `host` only beside the CLI; then `PATH` for the `hazync-` names only. It no longer searches the current
directory — before, a worker started in a writable directory could run a planted binary, and a planted
binary can print the canonical guest id. Setting `HAZYNC_HOST` mitigates (`run-workers.sh` refuses to start
without it); the guest-id checks (`run-workers.sh` and `selftest` at startup, `hazync run` only after a
rejected submission) do **not**, since they ask the binary. A source checkout refuses to POST to the default
public coordinator (`_guard_dev_writes`); `run-workers.sh` also runs a GPU smoke prove first.

**Alerts** (`hazync notify`, off unless set up) POST to the ntfy URL in `$HAZYNC_HOME/ntfy` (mode 600) or
`HAZYNC_NTFY`: handle, host name, and possibly a block id, log tail or coordinator error (≤ 3,500 bytes),
never the key. The topic is the only secret — knowing it lets anyone read alerts or post fakes;
`hazync notify new` makes an unguessable one; the ntfy server (`ntfy.sh` by default) sees every alert. A
hostile coordinator can inject text into a push, at most one push per problem per hour
(`HAZYNC_NTFY_REPEAT`). A hostile or unreachable ntfy server cannot stop work: `notify()` gives up after
10 s, never raises, and no launcher exits over an alert.

### 2.7 Verifier CLI and C ABI

`verify()` in `verifier/src/lib.rs` checks the receipt against the embedded `METHOD_ID_HEX`, `self_id`,
`KIND_RANGE` and the genesis pin, returning `Invalid` or `NotAnchored` (exit 1 or 2); no node, no network.
`verifier-ffi` exposes it as `hazync_verify_proof` (`include/hazync_verify.h`). Not trusted for most-work
or anything above `hi`; a checkpoint-anchored proof is `NotAnchored` by design. Whoever supplies the proof
cannot get a mid-chain or foreign-guest proof accepted; **whoever supplies the binary controls the answer**
(§2.9). The embedded id is held to canonical by `scripts/check-versions.sh`, and `release-sign.yml` asserts
the aarch64 verifier it builds embeds it.

### 2.8 Browser verifier: the served-module risk

`verifier-wasm/` is the same `verify` compiled to wasm (`verified`/`invalid`/`not_anchored`). **The risk is
what gets served, not what gets built.** Served at `hazync.org/verify/` (both files measured 200; wasm
1,065,400 bytes as of the v0.21.7 cutover, 2026-09-19). On 2026-08-11 the bitcoinghost.org module was two
re-baselines behind and told readers the live spine was forged; a stale module has the same size, exports
and HTTP 200, so only calling it shows the difference. `scripts/check-deployed-verifier.sh` (per
`HAZYNC_SITE`) requires a byte-identical loader, the release wasm size, `methodId()` equal to
`reproduce/METHOD_ID`, and that a one-bit-flipped spine fails. CI job `deployed-verifier` runs it on
schedule and `workflow_dispatch` only, so it catches drift after the fact. The signed CLI remains the
stronger check. Deploying both copies is part of [`BUILD_AND_RELEASE.md`](../BUILD_AND_RELEASE.md).

### 2.9 Release signing

`.github/workflows/release-sign.yml` (on `release: published` or dispatch with a tag). Binaries are built
outside CI except the aarch64 verifier (built from the tag, id asserted). It writes `SHA256SUMS.txt` over
the `hazync-*` assets present **when it runs** (failing if any would be left out), signs with
`GPG_PRIVATE_KEY`, uploads the sums and `.asc`. Fingerprint `777FE81F 8CC077FD 3D08055E 852C2B31 90F5B928`,
expiring 2028-04-25; second source `https://github.com/defenwycke.gpg`. Trusted for provenance, **not** for
how a binary was built: the guest id is reproducible, the host build is not attested, and any proof verifies
against the id whoever built the host. An attacker with write access can dispatch against any tag and get
whatever is attached signed. Tag injection into `run:` (H-1, exposed the key) is fixed (#57) and banned by
`scripts/check-workflow-injection.sh`. Swapping an asset after signing produces a checksum mismatch.

### 2.10 ghostd adoption

Consuming code is in `bitcoin-ghost/ghost` (PRs #543, #627, #630, #631), not this repo, and was not re-read
for this page. Trusted for letting a node skip validation or load a UTXO set on a proof's authority. SPEC
§11.2: positions form a permutation, coins are in bijection with the snapshot, rebuilt forest roots and
leaf count equal the proof's; it calls `verifier-ffi` rather than reimplementing anchoring.
`haze::HazyncAdoption` is obtainable only from `Authorise()` (armed + verified + dump matched). Eight
adversarial inputs — corrupt, truncated, empty, missing, non-genesis-anchored, each flag without the other
— elided nothing (#34). **Open:** a competing-chain proof and a reorg below the proven height;
`m_chain_tx_count` is a substituted lower bound (SPEC §14). Review targets in §5.1 item 6.

### 2.11 Websites

`https://hazync.org/` serves the board, a browser verifier, and the API proxied by
`coordinator/deploy/nginx-hazync.conf`; `coordinator/web/index.html` (`COORD_WEB`) passes handles through
`clean_handle` and escapes every sink (audit round 6). The site is built from the private
`hazync/hazync-web`, serves its own verifier copy pinned by sha256 in `deploy/deploy.sh` and
`tools/check.py`, and proxies `/api`. Trusted for nothing soundness-relevant except the verifier modules.
Whoever controls a site can publish false figures and instructions and serve a malicious verifier;
downloads stay checkable against the signed manifest and a second-source fingerprint.

---

## 3. Soundness argument and known gaps

### 3.1 Recursion (audit S1)

The chain/aggregation guests call `env::verify(self_id, prev_journal)` with `self_id` from host input, so a
malicious prover controls it. RISC0 composition binds an assumption `(image_id, journal)` the prover must
discharge, and the verifier checks the final receipt against the true `METHOD_ID` — but a nested
`self_id ≠ METHOD_ID` cannot survive top-level verification **only if every nested composition used the
true id**; relying on that implicitly is the trap. **Hardening:** every recursive step commits `self_id`
and asserts `prev.self_id == self_id` (`chain_step`/`aggregate`; a fold asserts `l.self_id == self_id &&
rr.self_id == self_id`, `main.rs:1502-1508`); the verifier asserts final `self_id == METHOD_ID`
(`verifier/src/lib.rs:100`). Together these force every level to the true id. `prove-chain-bad` folds a
block against a corrupted `self_id` and is rejected (positive chain VERIFIED, wrong-id chain REJECTED, on a
box). Recursive proofs are therefore sound on the same footing as single-block proofs, with no
honest-prover assumption, given §1.5.

### 3.2 Binding each proof to its block (2026-07 adversarial audit)

Four under-constrained witnesses, all fixed, each with a negative test (execute-mode `host adversarial` in
CI, or a GPU test) beside an honest baseline that must still pass:

- **H1 height.** `w.height == prev.height + 1`. An unbound height let a mining-capable prover set
  `w.height = 1`, disabling every soft-fork flag (segwit/taproot outputs anyone-can-spend) and inflating
  subsidy to 50 BTC while the journal committed the true height. (Range-fold was already bound by its
  genesis pin + adjacency.)
- **H2 segmented flags/witness.** Each chunk commits a per-input digest over `(raw_tx, input_idx,
  prevouts, coin metadata, flags)`; the aggregate recomputes it under the block's real flags. Before, a
  chunk could substitute a different valid spend of the same coin or use weaker flags. Test:
  `HAZYNC_H2_BADHEIGHT=1 host prove-seg`.
- **H3 in-block coins** must come from a strictly earlier tx and be spent at most once (before: double
  spend or spend-before-create).
- **H4 coinbase** runs real `CheckTransaction` + `IsCoinBase` (bad-cb-length, per-output `MoneyRange`,
  value-sum range).

A re-audit of those fixes found:

- **H5 multi-input fee prevouts (critical).** Only each input's own coin was authenticated, but
  `check_tx` sums the whole first-input prevouts blob, so a phantom coin inflated the fee (~21M BTC) or an
  omitted `BlockInput` skipped a script and a deletion. Fixed by a pre-pass tying the input list to each
  tx's real `vin`. Test: `host adversarial` #5 + honest 2-input baseline.
- **H6 genesis in-boundary (high).** `assert_genesis_in_boundary` pins the full boundary (`in_epoch_start`,
  `in_roots`, `in_recent`, `in_time` as well as tip/leaves/nBits); `verify-any` applies it whenever a range
  claims genesis. Closed a forgeable first-retarget difficulty and a phantom UTXO seed. `in_smt_root` joined
  later: #54 added it to `RangeState::is_genesis_anchored` but not to the standalone verifier (audit #3
  F-2) or the host gate (F-3), so a fabricated BIP30 history verified as genesis-anchored; both now require
  the empty-tree root, the verifier via the shared predicate.
- **H7.** The coordinator chains on the full boundary digest (height, tip, roots, leaves, nBits, time,
  epoch start, recent times, SMT root) with tip linkage, digest equality and height contiguity (H9,
  round 6), choosing the most cumulative work, not the tallest.
- **H8.** Every recursion-consumed journal (`ChainState`/`RangeState`/`ChunkOut`) starts with a domain tag
  asserted on decode.

### 3.3 Anchor identity and completeness (round 8, 2026-07-22)

Five-reviewer audit ([`history/AUDIT_2026-07.md`](AUDIT_2026-07.md)). Accumulator, FFI/VerifyScript
boundary and enforcement gating held; one verifier hole and five completeness deviations, all fixed except
the unprovable one:

- **A1/S5 anchor identity.** A bare `ChainState` committed no record of its anchor, so an `is_base=1`
  receipt on a fabricated anchor was indistinguishable from a genesis one — unreachable through shipped
  verifiers but a foot-gun. Fix: the guest commits `anchor_id = dsha256(base-anchor journal)`, and
  `verify-chain` pins it to `dsha256(genesis_anchor)`.
- **G3** witness commitment gated at segwit height 481824 (unconditional was a reject-valid stall);
  below it `unexpected-witness` forbids witness data, including the coinbase (G2).
- **G5** block-level `MoneyRange(nFees)` explicit, plus i128-safe `subsidy+fees`.
- **G1** BIP30 — then a bounded argument (sound to ~1,983,702); closed by #54 (§3.4).
- **N1/N2/N3** removed the dead host-flags field, length-prefixed the leaf `scriptPubKey`, made
  `tx_full_sigops` fail closed on a short blob.
- **G4** future-time stays unenforced.

Validated in execute mode (170→172 byte-exact; 130000/741000 VALID with byte-exact tips; 741000_badwit
rejected; check-bip30 PASS). `METHOD_ID` and leaf format changed, so re-proved from genesis.

### 3.4 Consensus-rule completeness

**Enforced** (real Core unless noted):

- Script validity, all types, per-height flags (P2SH/DERSIG/CLTV/CSV/segwit/taproot).
- `CheckTransaction` (structure, dup inputs, value bounds).
- No inflation: Σin ≥ Σout per tx; coinbase ≤ subsidy(height) + Σfees, overflow-safe; subsidy is the
  halving formula **reimplemented, differentially fuzzed vs Core**; block-level `MoneyRange(nFees)`.
- PoW (`CheckProofOfWorkImpl`, real `arith_uint256`) and retarget (real compiled `CalculateNextWorkRequired`,
  cross-checked against on-chain nBits at all 476 mainnet retargets).
- Merkle root including the CVE-2012-2459 mutation check (Core's `mutated` flag rejected); BIP141 witness
  commitment (activation-gated) and `unexpected-witness` below activation.
- Weight ≤ 4M; full sigop cost ≤ 80k (legacy + P2SH + witness).
- `time-too-old` (timestamp > MTP of previous 11).
- Absolute locktime (`IsFinalTx`); BIP68 height and time, the latter with real `MTP(coinHeight−1)`: the
  guest commits `mtp` on created-output leaves and host builders derive the same value (`block_mtp` window),
  validated by `check-ibd` genesis→550, `check-full` 741000, 170→172, and a real mainnet 90-day-CSV tx
  (`prover/test_bip68_real.sh`).
- Coinbase maturity (S2): the bridge sources real `coin_height` + `coin_is_coinbase` into `build_full`;
  validated on 741000 (`all_ok=true`).
- BIP34 (S4a): `check_bip34` compares the coinbase scriptSig height; 741000 `bip34_ok=true`.
- **BIP30 (#54):** in-block distinctness by sorted-txid check; cross-block by a coinbase-only sparse Merkle
  tree (key = coinbase txid, value = unspent output count), journalled as `in_smt_root`/`out_smt_root` and
  continuity-checked at the fold seam. Per block the guest proves the new coinbase absent-or-zero against the
  incoming root, inserts it, then decrements each spent coinbase — checking before updating, so a block
  cannot spend its own duplicate to zero and claim the slot free. The guest **derives** the coinbase txid,
  its output count and each spent coinbase's txid from the Core-deserialised transactions; only proofs come
  from the witness. `is_genesis_anchored` pins the tree empty. The ~1,983,702 ceiling is gone. The F3
  grandfathered overwrite remains (it deletes a superseded coin from the accumulator); those two blocks are
  ordinary inserts under the tree.

**Open (none architectural):** the standalone `build_full` harness's fabricated anchor uses placeholder
`recent_times`, so proving an isolated real time-locked block via `check-full` needs the real last-11
timestamps and per-coin MTP. The IBD/chain path has no such dependency.

**Not enforced:** G4 future-time (§1.4).

### 3.5 Milestones

1. ✅ S1 recursion hardening. 2. ✅ S2 real coin metadata. 3. ✅ BIP34 + BIP30. 4. ✅ Execute-mode
regression (`regress`, `check-full`: block 170 tip-match + adversarial inflation in seconds; standard
pre-flight before any GPU prove). 5. ✅ Block 741000 (670 inputs) proven on GPU: 16 chunk receipts →
aggregate, VERIFIED, tip byte-matches mainnet; exercised BIP141, BIP34/30, maturity/BIP68; fixed a header
builder that hardcoded version 1. 6. Scaling ([`history/HAZYNC_ARCHITECTURE.md`](HAZYNC_ARCHITECTURE.md)):
(a) ✅ succinct chunk receipts (2026-07-15; the 1645 s aggregate was dominated by lifting 16 composite
receipts); (b) ✅ archive-node bridge (§1.3); (c) parallel backfill → tree fold. Items 1-6 did not change the
trust base; it grew in v0.21.0 with patches 0012/0013.

### 3.6 Known gaps

- **No commissioned audit** (§5). Rounds 10 and 11 predate both libsecp patches.
- **Gate 4 not run:** no CORE build has rejected a corrupted signature inside the guest; the byte-flip
  control in `fuzz-native/realvector.cpp` runs natively on libsecp's stock backend, and the negative corpus
  has no signature case.
- **Accumulator reference fuzz control needs a rerun** (§6).
- The standalone `build_full` real-MTP anchor (§3.4).
- Most-work and ghostd competing-chain / reorg cases (§1.4, §2.10).

---

## 4. Durability: the image id and proof longevity

### 4.1 What makes a proof last

A receipt stays verifiable indefinitely provided we keep its method id **and** can rebuild the risc0
verifier that checked it; nothing expires on its own. What destroys a back catalogue is losing the record
of which id was current when, or the ability to reconstruct its verifier. The bytes are not the asset (a
proof is a reproducible fact about public data); the scarce thing is the recorded claim that a prover
proved block N first, so the **registry** is the asset. ⛔ A method id cannot be made mutable: it *is* the
commitment to the guest code.

Today is **Method 1**: one id, compiled into the verifier and the coordinator (which rejects anything else
as a build mismatch). A guest change retires the whole back catalogue. Its virtues: "verified" has one
meaning, one build produces accepted proofs, nothing has to be classified so nothing can be misclassified.
The risc0 pin `=3.0.5` is part of what must be rebuildable.

### 4.2 Append-only lineage — built (#244)

`reproduce/LINEAGE.tsv` records every canonical id in order with its risc0 toolchain: **17 ids, 16
supersessions, 2026-07-18 to 2026-09-07**. It is derived from git history of `reproduce/METHOD_ID` by
`scripts/lineage.sh`; `--check` fails if the committed file disagrees, so deleting a row does not delete the
evidence. It replaced prose that was incomplete (nothing superseded `7a8b29e0`, superseded in fact on
2026-07-26), pinned no risc0 version, and could not show a deleted row to `check-versions.sh`.
⚠ The lineage begins where the record does: `reproduce/METHOD_ID` was created 2026-07-18, 41 commits in;
earlier ids are not recoverable. ⛔ A shallow clone cannot reconstruct it and the check refuses to run — the
first generation, on a clone grafted at 412 commits, silently lost the two oldest rows (`d1fc4065` among
them). Gates: `scripts/lineage.sh --check`, `scripts/test-lineage.sh` (+ `--control`), both in CI.

### 4.3 An accepted set of ids — decided no (#244, closed 2026-09-16)

Operator decision: a verifier pins the canonical id. The shape proposed was `(method_id, risc0_version,
valid_from, valid_to)`, append-only (the lineage already supplies three columns). Why not:

- **Coverage grew as the guest was built**, so old ids assert less:

  | Rule added | Commit | Ids that predate it |
  |---|---|---|
  | P2SH sigop count guarded with `IsPayToScriptHash()` (#1) | `1ba1e10`, 2026-07-24 | 3 |
  | Two `cshims.c` corrections | `d5723a4`, 2026-08-02 | 9 |
  | BIP30 via coinbase-only SMT (#54, #83) | `70a0fa5`, 2026-08-03 | 10 |

  No rule has been added since 2026-08-03. **7 of 17 ids carry today's rule set** (`dfc9eeda` and the 6
  after it); **10 are development era**. Five of those six changes are performance/build (checkout-path
  independence, chunk payload encoding, segment distribution and parallel validation, the coprocessor
  field backend with ECMULT/codegen knobs, CORE becoming the shipped guest); `4722cec8` (audit #5) is
  hardening — `coin_leaf` bounds guards, a 32-bit-safe overflow check in `coinbase_value`, a missing
  `return true` (latent UB, never shown reachable) — closing paths without adding a rule. The experimental
  GHOST build, where speed levers land first, does not ship and cannot contribute to the board.
- **The soft-fork analogy does not transfer.** A soft fork is safe because a strict majority enforces the
  tighter rule; a proof system has no majority, and a prover picks the most permissive id on offer.
- **Widening only the verifier buys nothing for chains.** Composition is homogeneous, enforced inside the
  circuit (§3.1), so an old proof could be checked but never folded, and the spine would restart anyway.
  Preserving accumulated work needs the **guest** to accept earlier ids — moving the set inside the circuit,
  undoing the verifier-asserted S1 pinning, and requiring journal compatibility, which broke when
  `dfc9eeda` added the SMT roots to `RangeState` (an earlier note said `68819a54`; that changed the witness
  format, not the journal).
- **Entries would be permanent**, and since builds are reproducible a listed id that asserts less than
  believed could be rebuilt by anyone to make genuine proofs of a narrower claim: the set is as secure as its
  weakest entry. Any list must be compiled in, never fetched (a coordinator compromise would admit arbitrary
  ids).
- **"Purely internal" is a finding, not an observation.** `1d6c3792` left Core's code unchanged but made the
  aggregate *receive* wtxids, coin leaves and sequence numbers from chunks instead of recomputing them;
  showing the statement survived took two same-run differentials over 8,006 inputs plus a 16-chunk
  end-to-end on block 962,000, and the lock check was deliberately not relocated because a mistake would
  have been fail-open.

⚠ None of this is a live exposure: verifier and coordinator pin one id and the circuit pins composition.

### 4.4 If a re-baseline is ever forced (a position, not a plan)

Nothing is scheduled; building a survival mechanism would itself need a re-baseline. When a consensus bug,
security fix or worthwhile speed change forces a new id, the back catalogue is written off regardless —
that is the moment to consider **Method 2**: an accepted set that (1) **starts at the id being adopted**,
never reaching back, so its weakest member is the guest already in production, and (2) admits a new id only
if `new-valid ⊆ old-valid` (rules tighten, never loosen). Most changes are *equal* rather than stricter, and
equality is the harder claim, so the relation should be **tested, not asserted**: a gate running the candidate
and the guest in production over a corpus (done by hand once, for the since-superseded `1d6c3792`). The set would have to live inside the
guest. Open questions (put to [Discussion #299](https://github.com/hazync/hazync/discussions/299)): is the
start-and-tighten rule sound; what corpus would convince anyone a change did not loosen; is there prior art
for upgrading a zk circuit without invalidating its back catalogue; does putting the set inside the guest
change the answer; is Method 1 simply right. Method 2 is not assumed to win.

### 4.5 Lifting by recursion, and its limit

Old proofs could be **lifted** — the old verifier's execution proved inside the new guest; the machinery
exists. ⛔ Correction of an earlier claim: lifting preserves the *statement* ("guest X accepted this
block"), so it cannot add a check the old guest never made. A bespoke migration circuit could add just the
missing check, but for BIP30 that means rebuilding SMT state the old run never built. Lifting therefore
helps only the stable-rule era; **blocks proved under a narrower rule set must be re-proved**, which is the
correct outcome. Lifting remains interesting as reconciling old proofs *to* the current id, without a
permanent id list. ⏰ **Unmeasured and possibly decisive:** the cost of lifting a range versus proving it.

---

## 5. Review and audit status

**Nine rounds of self-audit and two external reviews** (rounds 10 and 11,
[`history/SECURITY_AUDIT_LOG.md`](SECURITY_AUDIT_LOG.md)). Both external passes were
**AI-assisted full-source reviews by people outside the project, not a commissioned professional audit**.
Neither found a soundness break in the guest, the verifier or the coordinator frontier. Only round 8 has a
standalone write-up ([`history/AUDIT_2026-07.md`](AUDIT_2026-07.md)); others are recorded where their
findings were fixed (the audit log indexed from `../SECURITY.md`, `reproduce/METHOD_ID`, code comments,
including our own mistakes and retractions). They found real defects, among them a canonical-chain break
(audit #3, F-1) that survived internal review because a comment, a test name and a fixture agreed with each
other and not with the chain. The two external passes independently placed the residual risk in the same
two places: the C++ bridge in the guest, and the accumulator. Everything has been reviewed; almost nothing
by someone who could be wrong in a different direction. No amount of test writing closes that (hazync#50
carried it through five internal audits).

### 5.0 Verify something first (about a minute, x86-64 Linux, no GPU, no chain data)

```sh
curl -fsLO https://github.com/hazync/hazync/releases/latest/download/hazync-verify-x86_64-linux-gnu
chmod +x hazync-verify-x86_64-linux-gnu
curl -f https://api.hazync.org/api/spine/proof -o spine.snark
./hazync-verify-x86_64-linux-gnu --json spine.snark
```

Expect `"verified": true`, `"genesis_anchored": true`, the image id and the out-boundary chain state. Then
check `tip_hash` (display order) against an independent source — when written it matched
`blockstream.info` exactly; if it ever does not, nothing else here matters — and check the image id against
`/api/meta` and `reproduce/METHOD_ID`. `epoch_start_time` is the first block of the tip's retarget period
(`rs.out_epoch_start`), equal to `1231006505` only below height 2016; the genesis pin applies to the
in-boundary, which `--json` does not print. The spine is short and is not the board's `frontier` (the
coordinator chaining many ranges, not checkable in one download).

### 5.1 What needs outside eyes, in order

1. **Clean-machine reproducible build** — highest value, lowest cost. Nobody outside has re-derived
   `METHOD_ID`: `docker build -f reproduce/Dockerfile .` must print `reproduce/METHOD_ID`. Five audits checked
   the pinning web (toolchain, Core, secp256k1, Cargo.lock); none rebuilt the id. If it does not match, no
   other check would notice. The container is the only canonical derivation; a local build legitimately
   differs (it embeds `CARGO_HOME`).
2. **The accumulator** (`accumulator/` and the guest `Stump` port; formerly hazync#50) — the one failure that
   **reaches backwards**: every proof against a broken commitment is worthless and repair is re-proving from
   genesis. Internal coverage: exhaustive single-delete equivalence vs a full-forest oracle for every n ≤ 40
   and every index; exhaustive cached-vs-pre-cache equivalence across deletes with follow-up operations and
   power-of-two neighbourhoods; 192k-run differential of the cached `Forest`; ~893k-exec libFuzzer campaign
   with a positive control; SEC-2 (`delete` no longer trusts an unverified position). All self-audit, all
   small-n, against a board heading for ~200M leaves, and new code. The guest's `utreexo.rs` is the
   authority (it carries hardening the host oracle does not); `Stump` is the readable spec.
3. **Patches 0012/0013** (§1.2) — the newest consensus-relevant code; no outside review has seen it (rounds
   10-11 predate it by about a month). Check the `lift_x` argument independently, including that a hostile or
   truncated table can only cost the fallback.
4. **The C++ bridge** `prover/methods/guest/verify_input.cpp` — our glue (deserialisation, bounds, canonical
   leaf construction, `core_*` exports pinning constants). The zkVM has no memory protection, so an
   out-of-bounds read is a silent wrong answer. Audit #5 found one unguarded index (L-1, fixed, no exposure).
5. **The coordinator under a hostile-coordinator model.** Reviewed so far with `COORD_URL` operator-chosen
   and trusted. Since hazync#69 (closed 2026-08-24) anyone can run a coordinator ([`COORDINATOR.md`](../COORDINATOR.md)),
   seed from peers and set `PEER_COORDINATORS`, so the worker's handling of responses (unbounded `get()`,
   claim responses, anything assuming well-formed answers) needs a second pass.
6. **ghostd integration** (hazync#31/#42/#46 closed; ghost PRs #543/#627/#630/#631) — **start here: a
   prover bug yields a proof nobody accepts; a bug here accepts one nobody should.** Test that
   `HazyncAdoption` (private constructor; reaches `ActivateSnapshot` as an explicit argument, so
   `loadtxoutset` cannot borrow a proof's authority for an arbitrary file) has no path that skips a check;
   that nothing in the snapshot path still trusts a developer-chosen constant where Core's height/hash
   chainparams dependencies were replaced by proof authority and accumulator roots, and that where
   chainparams knows a height its hash is still checked (disagreement must be loud); the
   `m_chain_tx_count = height + 1` substitution (only non-zero-ness is load-bearing; progress
   under-reports); rebuilt blocks carrying real txids so `DisconnectBlock` lookups do not silently miss;
   and each `IsHazeMode()` site (*node hazed* vs *block stripped* was conflated three times — genesis marked
   stripped, a fresh hazed node could not start while unit tests passed). **Not established:** B4
   (adopted chainstate byte-identical to full validation) only at low height; CI does not build ghost-core
   (green ticks are the Rust workspace); the hazed→hazed receive guard is unreachable in practice. The
   ghost repo's `test/hazync/` harnesses found every substantive bug.

**Deliberately excluded:** Core's consensus code itself — pinned tag, only portability patches 0001/0002,
far more review than could be commissioned; the pinning is item 1.

### 5.2 Open items (verified 2026-09-14)

1. Field backend gate 4 not run (§3.6).
2. Accumulator reference fuzz control needs a rerun (§6).
3. **CORE fleet figures rest on few measurements.** [`OVERVIEW.md`](OVERVIEW_CORE.md) (G2) gives **44–73 L40S
   card-years**, INFERRED from one near-tip block (966,108, ~0.77 card-s per input);
   [`BUILD_AND_RELEASE.md`](../BUILD_AND_RELEASE.md) computes 10 cards from block 962,000 proved serially, and
   its 8 × L40S check on 966,108 puts a sub-10-minute block at ~13.
4. ✅ Decided: no accepted set of method ids (§4.3).
5. Worker reads coordinator responses unbounded (§2.6).
6. No commissioned external audit; priorities: accumulator, recursion binding, ghostd adoption.
7. Unsigned claims (#310), many fresh keys unlimited, no per-address cap behind the web box (§2.5).
8. ✅ Fixed: stolen key moving attribution (#311) — returns if `ROTATE_ENABLED=1`.
9. ✅ Fixed: prover search in the current directory (#312).
10. ghostd competing-chain and reorg cases open (§2.10).

---

## 6. Fuzzing

Components reachable without a GPU or full Core build. Each pass: an **independent oracle**, a **positive
control** proving the harness catches the bug class, an honest scope note. Detailed reports:
[`../audit-fuzz/FINDINGS.md`](../../audit-fuzz/FINDINGS.md), [`../coordinator/SEAM_FINDINGS.md`](../../coordinator/SEAM_FINDINGS.md).

| # | Target | Harness | Scenarios | Result |
|---|---|---|---|---|
| 1 | Utreexo guest hardened `delete`/`verify` | `audit-fuzz/` (libFuzzer + `arbitrary`) | ~893k execs | **clean** |
| 1c | Reference `Stump` positive control | same | — | ⚠ **needs a rerun** |
| 1d | Cached `Forest` vs pre-cache (`forest_cache_equivalence`) | same | 192k | **clean**, mutation-checked |
| 2 | Coordinator `_frontier_chain` (S1/F1/H9) | `coordinator/seam_fuzz.py` | 200k | **clean** |
| 2c | Seam control, H9 guard removed | `--control` | — | caught (expected) |
| 3 | `parse_range`, `clean_handle`, `is_hex` | `coordinator/parse_fuzz.py` | 300k | **clean** |
| 4 | `block_script_flags`, `add256`, `median_time_past` | `guest-pure-fuzz/` | 700k+ | **clean** |
| 5 | Guest C++ shim + Core TUs natively vs references | `fuzz-native/differential.cpp` (#222) | 27 checks | pass |
| 6 | Same FFI glue under ASan+UBSan (truncated/empty/bit-flipped/random) | `fuzz-native/memsafety.cpp` | 661 cases | 0 findings |
| 7 | Real mainnet inputs via `verify_input`, byte-flip control | `fuzz-native/realvector.cpp` | 892 inputs | all valid |
| 8 | In-zkVM negative corpus, one rule violated at a time | `fuzz-native/negative-corpus.sh` (#223) | 7 rules | **7/7 refused** |
| 9 | Leaf preimage bytes: C++ spend/create sides vs Rust | `leaf-differential/` | — | agree |

Counts are manual campaigns. CI runs `seam_fuzz.py 20000`, its `--control`, `parse_fuzz.py 30000`
(`.github/workflows/adversarial.yml`), the `audit-fuzz` and `guest-pure-fuzz` unit tests and
`leaf-differential`. The libFuzzer campaigns (1, 1c, 1d) and `fuzz-native/` (5-8) do **not** run in CI.

- **1 Accumulator.** A `Forest` oracle in lockstep with the guest's `Stump`, driven by honest ops,
  tamper-from-honest forged spends, fully attacker-authored proofs and arbitrary `verify` calls; asserts
  soundness, atomicity, completeness, no-panic each op. Three campaigns, zero failures. Control as recorded:
  the then-unhardened reference crashed on the SEC-2 location-confusion class in <1 s; the guest rejected
  that input in 1 ms. ⚠ #63 (`8e789a9`) hardened the reference and removed the `tree_of` panic the control
  relied on; whether `delete_soundness_reference` still detects its class is unverified. The in-crate L-2
  tests have their own control, verified 2026-08-02.
- **2 Seam.** A randomized model-checker over a tiny symbolic alphabet drives the real DB-backed function
  against an independent DFS oracle of every legitimate genesis-anchored seam path: zero over-reports,
  splices or crashes; removing H9 is caught in the first scenarios.
- **3 Inputs.** Claim-id aliasing/bounds, the stored-XSS choke point: no crash, invariants held.
- **4 Guest helpers.** `build.rs` extracts the functions and constants verbatim from guest sources (zero
  drift). Activation heights (DERSIG 363725, CLTV 388381, CSV 419328, NULLDUMMY 481824) match mainnet
  chainparams with OFF at h-1, ON at h, plus BIP16/Taproot exception hashes, retroactive base flags,
  monotonicity; C++-side flags (`verify_input.cpp`) out of scope. `add256` vs a u128-limb reference, 500k
  random + carry-edge vectors; `median_time_past` sorted-middle + order-independence, 200k windows. On
  2026-07-27 this harness had silently stopped building (functions moved to `script_flags.rs`) because
  nothing ran it; fixed and added to CI.
- **5-8 Native/in-zkVM.** `fuzz-native/build.sh` compiles the guest C++ shim and Core TUs for the host,
  overlaying pristine copies of the two files patches 0001/0002 change. Differential covers subsidy,
  retarget clamps, CVE-2012-2459 mutation and cumulative work. The negative corpus violates nBits, weight,
  sigops, subsidy, merkle, PoW and BIP34 against a positive control, in execute mode.

**Not covered** (needs the prover or the Core build; right targets for an external audit):

1. **`verify-any` boundary-digest binding** — the seam argument assumes it commits the full boundary (UTXO
   roots, difficulty, MTP, tip) collision-resistantly; seam fuzzing cannot see an under-bound field.
2. **Core-in-zkVM parity** — partly covered by 5-8. Not covered: the native harnesses use libsecp's stock
   field backend, not 0012/0013; the negative corpus has **no bad-signature case** (gate 4), nor premature
   locktime or BIP30; nothing in `fuzz-native/` runs in CI.
3. **Recursion / `METHOD_ID` fold binding** — that a doctored image or spliced level never yields an
   accepted tip; `seam_fuzz.py` models it only at the coordinator layer.

Gotchas: `cargo-fuzz` needs nightly, use `-rss_limit_mb`; keep native/CMake builds at `-j2` (`cc1plus`
OOMs above); the guest C++ needs the risc0 riscv toolchain only for the guest build, a host differential
uses system g++ with the same file list.

```bash
# from the repo root
cd audit-fuzz && cargo test --release
cargo +nightly fuzz run delete_soundness            -- -max_total_time=120   # clean
cargo +nightly fuzz run delete_soundness_reference  -- -max_total_time=60    # control: unverified since #63
cd coordinator
python3 seam_fuzz.py 200000     # clean
python3 seam_fuzz.py --control  # control: caught
python3 parse_fuzz.py 300000    # clean
cd guest-pure-fuzz && cargo test --release
```

---

## 7. libzync

libzync is a new core with its own proof statement, image ids and review history; nothing in sections 1-6
should be read as describing it.

- What a libzync proof states and every check a verifier makes:
  [`libzync/SWIFTSYNC_PROOFS.md`](../libzync/SWIFTSYNC_PROOFS.md) (§2 what is proven, §5 verifier procedure).
- Consensus rules the libzync guest enforces against Bitcoin Core v28.0, and what is still our own code:
  [`libzync/CONSENSUS_RULES.md`](../libzync/CONSENSUS_RULES.md).
- Review rounds and fixes: [`libzync/LIBZYNC.md`](../libzync/LIBZYNC.md) §7.
