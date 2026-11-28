# R3-1: OpenVM circuit-level guarantees Hazync relies on (28.11.2026)

Reviewer 1 of round 3, read-only. Hazync at a9b78510 (`r3tree`). OpenVM v2.1.0 at `~/.cargo/git/checkouts/openvm-e4fe658744a82733/05f3b73` (`O/` below), stark-backend v2.1.0 at `.../stark-backend-708f1155344bec0c/a6b4761` (`B/`). Hazync paths are relative to `r3tree/`.

## Verdict

**No Critical, High or Medium finding.** Every guarantee in the question is enforced by an AIR constraint, not only by the executor. That holds as long as Hazync keeps its current preconditions, and today it does. Two of those preconditions carry real weight: `x1 != x2` before `add_ne`, and a non-identity input to `double`. If a future edit breaks either one, a prover can forge the MSM result (R3-1 #1).

The other items below are informational notes. After them comes a list of what I could not verify myself.

---

## Q1: secp256k1 and modular chips

### How the arithmetic is constrained (mod-builder `FieldExpr`)

- **Constraint.** Every op is one or more relations `expr(inputs, vars) = q·p` over the integers.
  - It is checked limb by limb with carries: `O/crates/circuits/primitives/src/bigint/check_carry_mod_to_zero.rs:78-103` and `check_carry_to_zero.rs:69-109`.
  - The modulus limbs are **AIR constants** (`check_carry_mod_to_zero.rs:38-47`).
- **Quotient limbs** are range-checked to `[-2^8, 2^8)` (`check_carry_mod_to_zero.rs:79-88`).
- **Carries** are range-checked to `carry_abs_bits` (`check_carry_to_zero.rs:87-96`).
- **No field wraparound.** `carry_abs_bits + limb_bits < F::bits() - 1` is asserted at keygen (`:80-84`). The top carry is forced to 0 (`:107-108`). So the field equation is an integer equation, and the result is correct mod p.
- **Output limbs** (`vars`) are range-checked to 8 bits (`O/crates/circuits/mod-builder/src/builder.rs:437-448`). They are **not** checked to be below p. That is by design (`O/extensions/algebra/moduli-macros/src/lib.rs:123-129`).
- **Input limbs** are not range-checked in the chip. They come from RV32 memory, which stays bytes because every writer range-checks what it writes:
  - FieldExpr vars, above;
  - SHA `final_hash`: `O/crates/circuits/sha2-air/src/air.rs:193-197`;
  - hint store: `O/extensions/rv32im/circuit/src/hintstore/mod.rs:219-236`;
  - deferral OUTPUT bytes: `O/extensions/deferral/circuit/src/output/air.rs:313-320`;
  - deferral CALL: `call/air.rs:117-157`.
- **Opcode binds the flags.** `expected_opcode = Σ flag·opcode` (`O/crates/circuits/mod-builder/src/core_chip.rs:137-156`), and the flags are boolean with their sum boolean. A prover cannot run `mul` for `div`, or SETUP for an op.
- **SETUP is not constrained in-circuit** (`builder.rs:371-376`, a TODO). This does not matter for soundness: the modulus (and the curve's `a`) are AIR constants. Missing setup only blocks the honest executor.

### add_ne (`O/extensions/ecc/circuit/src/weierstrass_chip/add_ne/mod.rs:29-49`)

- The constraints are `λ·(x2−x1) ≡ y2−y1`, `x3 ≡ λ²−x1−x2` and `y3 ≡ λ(x1−x3)−y1` (mod p).
- **Degenerate case** (header comment, `:29-30`):
  - if `x1 ≡ x2` and `y1 ≢ y2`, the row cannot be satisfied. This is a completeness failure, not a soundness one;
  - if `x1 ≡ x2` and `y1 ≡ y2`, **λ is free**, and so are x3 and y3.

### double (`double/mod.rs:34-60`)

- The constraint is `λ·(2y1) ≡ 3x1² + a` (the denominator becomes 1 only on the SETUP row, where the flag is 0 and is tied to the opcode).
- **Degenerate case:**
  - on (0,0), the identity's encoding, λ is free;
  - `y1 = 0` cannot occur on secp256k1, because its group has prime order and so no 2-torsion point.

### Modular div (`O/extensions/algebra/circuit/src/modular_chip/muldiv.rs:37-42`)

The constraint is `z·y ≡ x`. When `y ≡ x ≡ 0`, z is free. Hazync divides only by a libsecp `s` it has already checked to be non-zero (`secp-0201-openvm-batch-liftx.patch:22-29`).

### is_eq / assert_reduced (`O/extensions/algebra/circuit/src/modular_chip/is_eq.rs:131-261`)

- **What is constrained:**
  - `cmp_result` comes from `IsEqArraySubAir` on the raw limbs (`:141-147`);
  - **both** operands are constrained below N through `lt_marker` and the `b_lt_diff`/`c_lt_diff` range check (`:155-257`). The diffs are range-checked to `[1, 256]` (`:255-261`), gated on `is_valid − is_setup`.
- **Casework.** I checked it by hand for `c_lt_mark = 1` and for `c_lt_mark = 2` with both orderings of the two diff indices. In every case, the limbs above each operand's diff index are forced equal to N, and the limb at that index is forced strictly smaller. **Sound.**
- **Consequences:**
  - `assert_reduced` is `black_box(PartialEq::eq(self, self))`, which is one `is_eq`, so it is a real circuit check (`moduli-macros/src/lib.rs:530-536`);
  - every `==` on IntMod (`:743-748`), and on points (derived field-wise, `O/extensions/ecc/sw-macros/src/lib.rs:99`), forces both sides canonical;
  - byte equality then equals residue equality.

### from_xy_nonidentity (`O/extensions/ecc/guest/src/weierstrass.rs:125-135`)

- `y·y != x³ + a·x + b` is computed by the mul and add chips and compared with `is_eq`. So the on-curve check is a circuit check.
- (0,0) is rejected because 0 ≠ 7.
- Hazync feeds it canonical coordinates (`Fq::from_le_bytes`, which checks `is_reduced` in RV32 code: `moduli-macros/src/lib.rs:406-428`). See `engine/guest/batch_ecdsa_ovm.rs:73-76`.

### Hinted sqrt and non-QR (`moduli-macros/src/lib.rs:850-905, 933-975`)

- **Zero** short-circuits before any hint (`:875-877`).
- **The hint's sqrt** is `assert_reduced`.
- **QR claim:** `sqrt² == self`.
- **Non-QR claim:** `sqrt² == self·non_qr`.
- **The `non_qr` itself:**
  - it is hinted and `assert_reduced`;
  - `exp = −1/2` is `assert_reduced`, which makes it exactly the integer (p−1)/2;
  - then `non_qr.exp_bytes((p−1)/2) == −1`, which is Euler's criterion (`:944-960`). `exp_bytes` is a correct base-4 left-to-right ladder (`O/extensions/algebra/guest/src/exp_bytes.rs:7-31`);
  - it is cached per run in a `OnceBox` (`:971-975`).
- **A bad hint** loops forever, so there is no terminating proof.
- **Sound.** Hazync's `hz_fe_sqrt` (`engine/guest/edits.py:141-149`) panics on a non-canonical input and returns `to_be_bytes()`, which asserts the result reduced.

### Hazync's side of the preconditions

These are the call sites that decide whether the degenerate cases above can be reached.

- **`add_into`** (`engine/guest/msm_ovm.rs:208-227`) runs `add_ne` only after `eq_impl` on x says "different".
  - `eq_impl` is sound, and needs both x values canonical. Chip outputs are canonical in honest runs; if one is not, the proof fails and nothing is forged.
  - Equal x splits two ways: `y1 + y2 == 0` gives the identity; otherwise the points are equal and it doubles.
  - `filled` tracks the identity exactly.
- **Doublings** happen only when `acc_f` is set (`msm_ovm.rs:288-292, 351-356`).
- **Identity bases** are dropped with `is_eq` (`:254`).
- **The Straus table** (`:336-347`) never reaches the identity, since k+1 ≤ 8 < n.
- **The live-coin accumulator `Acc`** uses the complete `add_assign_impl::<true>` (`batch_ecdsa_ovm.rs:148-150`; `O/extensions/ecc/sw-macros/src/lib.rs:282-310`).
- **Points on the curve:**
  - Q is parsed by libsecp inside the VM;
  - R passes `from_xy_nonidentity`;
  - G is a constant;
  - the ECMH points are checked `y² = x³+7` (`prover/methods/guest/src/swiftsync.rs:85-95, 123-126`).
  - So no invalid-curve inputs reach the formulas.
- **Every byte that carries meaning** is read through a canonicalising accessor:
  - `as_le_bytes` / `to_be_bytes` assert reduced: `O/extensions/algebra/guest/src/lib.rs:187-190`; `weierstrass.rs:24-28`;
  - this covers the transcript bytes (`batch_ecdsa_ovm.rs:80-82`), `accept_r`'s x and parity (`:38-45, 70, 77`), `from_p` (`:134-136`) and the shim's `as_bigint` (`engine/platform/risc0-crypto/src/lib.rs:17-20`);
  - `reduce_le_bytes` (kind 0) ends in `assert_reduced` (`moduli-macros/src/lib.rs:797-815`);
  - MSM scalars are read unchecked (`msm_ovm.rs:255`), but they are asserted reduced first (`batch_ecdsa_ovm.rs:67, 122`). An unreduced scalar would give the same point anyway, since the points have order n.

### R3-1 #1: the add_ne / double / div degenerate cases are load-bearing guest invariants

- **Severity:** Informational today, latent High. **Confidence:** high.
- **Evidence:** `O/.../add_ne/mod.rs:29-49`, `double/mod.rs:46-56`, `modular_chip/muldiv.rs:37-42`, against the guards at `msm_ovm.rs:217-225` and `:288-292, 351-356`.
- **Attack, if a guard is removed or bypassed.** Suppose a later "optimisation" calls `add_ne_assign_nonidentity` on equal points, which in Pippenger means two equal bucket entries. Or suppose it doubles the (0,0) encoding.
  1. Then λ is free.
  2. The prover picks λ so that the running sum lands where it wants it. For example, it can make `Σ z·u2·Q − Σ z·R + (Σ z·u1)·G` come out as the identity for a forged signature. It needs only one free row to cancel the rest.
  3. `finish()` then returns true (`batch_ecdsa_ovm.rs:124`).
- **Recommendation:** keep the `eq_impl` x-test and the `filled` / `acc_f` flags as stated invariants of `msm_ovm.rs`. Never call the `*_nonidentity` intrinsics outside `add_into` or the `acc_f`-guarded doublings. A fuzz test with repeated keys and repeated R points would catch a regression here.

### R3-1 #2: chip outputs are not canonical; the readers are

- **Severity:** Informational. **Confidence:** high.
- **Evidence:** `builder.rs:437-448` constrains only 8-bit limbs. For secp256k1 p, a non-canonical encoding exists only for values below 2^256 − p (about 2^32). Hazync's code above never reads raw output bytes without a reduction assert. Any new code that compares or hashes `as_le_bytes_unchecked()` of a chip output would let a prover choose between two encodings of one residue.

---

## Q2: the deferral / in-guest verify path

The guest calls `verify_stark_unchecked::<0>(&commit)` (`engine/platform/risc0-zkvm/src/lib.rs:165`). It then checks `sha256(exe‖vm) == image_id` and `user_public_values == sha256(journal)` (`:166-176`).

### Is the child's recursion key fixed? Yes

The relevant AIR is `DeferredVerifyPvsAir` in `O/guest-libs/verify-stark/circuit/src/verifier/air.rs`.

- **The key's pre-hash is fixed.** The verifying-key pre-hash of the proof being verified is received as the **AIR constant** `expected_internal_recursive_vk_commit.vk_pre_hash` (`:240-250`).
- **The cached commit is chosen by depth** (`:212-238`):
  - at depth > 1 it is the child's `internal_recursive_vk_commit`, constrained equal to the constant (`:257-261`);
  - at depth 1 it is the child's `internal_for_leaf_vk_commit.cached_commit`, with `internal_recursive_vk_commit` forced unset (`:252-255`). That value is hashed into `app_vm_commit` together with the app and leaf VK commits (`:265-283`). `app_vm_commit` then goes into Hazync's image id.
  - The depth-1 selector (`:143-150`) is sound. If d ≠ 1, the inverse column is forced to make `is_depth_not_one = 1`.
  - The depth is range-checked to `[1, 256]` (`:152-157`), matching the host's `MAX_RECURSION_DEPTH` (`O/crates/verify/src/pvs.rs:13-14`).
- **The other checks** are `internal_flag == 2` (`:140`), `exit_code == Success`, `is_terminate == 1` (`:133-135`) and `def_idx` constant (`:364`).
- **These mirror the host verifier** check for check (`O/crates/verify/src/lib.rs:131-245`).
- **A child that verified grandchildren:**
  - `deferral_flag` must be 0 or 2 (`:483-484`);
  - when it is 2, `def_hook_commit` must equal the AIR constant (`:485-490`);
  - when it is 0, everything is forced zero, including `def_hook_commit` (`:497-501`; this is audit finding #12, fixed);
  - the accumulator paths go to the root `DeferralAccMerklePathsAir`, which requires DEFERRAL_AS siblings to stay unchanged (`O/crates/continuations/src/circuit/root/def_paths/air.rs:161-190`; this is the in-circuit side of audit #01 and #02).

### Are the child's public values bound to what the guest reads? Yes

1. The child's `final_root` goes to `UserPvsInMemoryAir` (`verifier/air.rs:167-173`). That Merkle-proves the user public values in the child's final memory. Their count is fixed at keygen (`num_user_pvs`).
2. `DeferralOutputCommitAir` sponge-hashes `[def_idx, len] ‖ exe ‖ vm ‖ pvs`. It uses **canonical** 4-byte decompositions (`O/guest-libs/verify-stark/circuit/src/output/air.rs:79-90, 98-160`), and the result becomes the deferral output commit.
3. In Hazync's VM, the OUTPUT chip writes the bytes into guest memory. They are range-checked, and they must sponge-hash to that commit with `output_len == section_idx·8` (`O/extensions/deferral/circuit/src/output/air.rs:196-223, 313-320`).
4. Hazync compares all 32 bytes. Because the encoding is canonical, no other 4-byte string decodes to the honest field values.

### R3-1 #3: `output_len` is an unauthenticated hint until OUTPUT runs (zkSecurity v2.0.0 finding #10, unchanged in 2.1.0)

- **Severity:** Informational for Hazync. **Confidence:** high.
- **Evidence:** `verify_stark_unchecked` branches on `output_len` before OUTPUT (`O/guest-libs/verify-stark/guest/src/lib.rs:17-26`). It only panics or allocates on it, and OUTPUT then authenticates the length.
- **Impact:** a malicious prover can only make its own run fail. Nothing to do, unless Hazync ever reads `output_len` itself.

### R3-1 #4: the deferral input commitment is chosen by the prover

- **Severity:** Informational. **Confidence:** high.
- **Evidence:** `engine/platform/risc0-zkvm/src/lib.rs:72-79, 162-165`.
- **Why it is safe:** the commitment only names which proved deferral result to fetch. The statement is bound by the image id plus the public-value checks above. Attaching the same valid child twice is accepted. That is harmless for soundness; whether a duplicate matters to the aggregate's logic is a question for the guest reviewers.

---

## Q3: the SHA-256 chip

- **The chip only compresses:** `zkvm_sha256_impl(state, block, out)`. Padding is guest RV32 code, constrained like any other instruction. I checked the padding by hand:
  - the shim: `engine/platform/sha2/src/lib.rs:77-108`;
  - Core's in-place `Finalize`: `engine/guest/core-0202-sha-finalize-in-place.patch:16-34`. The `bytes` accounting matches the original `1 + (119 − n) % 64 + 8`;
  - `hz_d64`'s fixed pad blocks: `core-0201...patch:142-158`, length 512 and then 256.
  - All are correct.
- **Main chip** (`O/extensions/sha2/circuit/src/sha2_chips/main_chip/air.rs`):
  - it reads `prev_state` and the 64 message bytes from memory and sends STATE, MSG1 and MSG2 under one `request_id` (`:111-190`);
  - `request_id` starts at 0 and goes up by 1 per row (`:111-127`);
  - all three pointers have their top byte range-checked (`:226-238`);
  - it writes `new_state` through the memory bridge (`:309-337`).
- **Block hasher** (`.../block_hasher_chip/air.rs:55-158`):
  - MSG1 and MSG2 are received on rows 0 and 2 as bytes composed from boolean `w` bits, with `request_id` constant across the block (`:144-158`);
  - STATE (`prev_hash`, `final_hash`) is received on the digest row;
  - this is a permutation bus and the main chip's ids are distinct. So each block must take all three messages of exactly one request. A count mismatch means the bus does not balance.
- **Core AIR** (`O/crates/circuits/sha2-air/src/air.rs`):
  - a and e bits are boolean on every row (`:135-139`);
  - the message-schedule `w` bits are boolean on every round row, including the wrap-around into row 0 (`:507-512`);
  - for `W_t = σ1 + W_{t−7} + σ0 + W_{t−16}`, carries are boolean in pairs from row 4 on (`:458-506`), and the helper equalities are pinned (`:402-455`);
  - the digest row has `prev_hash + work_vars = final_hash` mod 2^32, with boolean u16 carries, and the bytes are range-checked (`:144-198`);
  - the private bus links each block's `prev_hash` to the previous digest row's state bits, which seed that block's first rounds (`:290-322, 327-385`);
  - trace shape: a round row cannot be followed by padding, the last row is padding, and row indices step correctly (`:217-266`).
- **I found no way to get a wrong digest accepted.**
- **Prior review:** this AIR was in the zkSecurity phase-4 scope, with a Lean4 formal-verification supplement for the new SHA-2 and Keccak AIRs (audit report p.2).
- **Audit finding #24** (the `ptr_max_bits` precondition is not asserted) does not apply. Hazync bounds `pointer_max_bits` to 24..=29 (`engine/host/src/engine.rs:96-102`), and the chip asserts ≥ 24 (`main_chip/air.rs:227`).

---

## Q4: buses Hazync's statements depend on

| Bus | What depends on it | Balancing / constraint I checked |
|---|---|---|
| Memory (offline checker and Merkle) | every byte the guest reads or writes, the byte-memory invariant, the user public values in final memory | not re-derived (see below) |
| Execution / program | the instruction that ran is the one in the committed program | not re-derived |
| VariableRangeChecker / BitwiseOperationLookup | the limb, quotient and carry bounds behind every FieldExpr; the is_eq diffs; SHA bytes; pointer bounds | each sender is gated by `is_valid` (cited above); the lookup chips themselves were in phase-1 scope |
| SHA-2 permutation bus | message and state binding between the main chip and the block hasher | argued above |
| Deferral (Poseidon2 / count / accumulator) | input → output binding of `env::verify` | CALL and OUTPUT AIRs read; aggregation and hook not re-derived |
| Recursion PublicValues / CachedCommit / PreHash / FinalTranscriptState | child statement binding inside verify-stark | receive sides read (`verifier/air.rs:176-250, 366-380`) |

- **Backend:** balancing is enforced by SWIRL's LogUp-GKR fractional sum. Audit finding #04 (bus-index `u16` overflow) is fixed in the backend Hazync uses: `B/crates/stark-backend/src/verifier/batch_constraints.rs:358` computes `from_u16(bus_index) + ONE`.
- **Host side:** audit fixes #01, #02 and #09 are present (`O/crates/verify/src/deferral.rs:66-93`, and the Merkle proofs are verified for both flags at `O/crates/verify/src/lib.rs:316-325`).

---

## What I did not verify (stated plainly)

1. **The proof system itself is not re-derived:** SWIRL, LogUp-GKR, ZeroCheck, stacking, WHIR, and the 39-AIR recursive verifier circuit, including the Poseidon2 AIR. I relied on the zkSecurity v2.0.0 audit (phase 1), on the fixed findings I spot-checked (#01, #02, #04, #09, #12), and on its own soundness analysis.
2. **The memory argument** (offline checker, Merkle, boundary) and the execution and program buses: I assumed them. The "RV32 memory holds bytes" invariant rests on every writer range-checking. I checked the writers Hazync's VM config uses (rv32im hint store, FieldExpr, SHA-2, deferral CALL and OUTPUT) but not every rv32im core chip.
3. **The chain that ties the verify-stark circuit's own AIR constants** (`expected_internal_recursive_vk_commit`, `expected_def_hook_commit`) to the outer statement. The outer proof pins `def_hook_commit` through the baseline in the pinned key file (`engine/host/src/segpar.rs:262`; `engine/verify-core/src/lib.rs:5-15, 183-188`). I did **not** trace the hook and deferral-aggregation circuits to confirm that `def_hook_commit` commits to the deferral circuit's full VK, constants included. The SDK builds it so (`O/crates/sdk/src/prover/deferral/agg.rs:195-252`). Confidence medium.
4. **Audit finding #00** (padded deferral aggregation children): the client calls the fix "partial", with domain separation in PR #2857. I saw `is_present` gating in `O/crates/continuations/src/circuit/deferral/inner/def_pvs/air.rs:118-223` but did not re-derive it.
5. **`IsEqArraySubAir`, `CanonicitySubAir` and the Rv32VecHeap adapter:** read at the call sites only; their bodies were not re-derived.
6. **Nothing was executed.** No test, poc or fuzz run; this report is from reading code.

## Files read

- **Hazync:**
  - `engine/guest/{batch_ecdsa_ovm.rs, msm_ovm.rs, edits.py, secp-0201-openvm-batch-liftx.patch, core-0201-sha-openvm-native.patch, core-0202-sha-finalize-in-place.patch}`;
  - `engine/platform/{risc0-zkvm/src/lib.rs, risc0-crypto/src/lib.rs, sha2/src/lib.rs}`;
  - `engine/host/src/engine.rs`;
  - `engine/verify-core/src/lib.rs` (head);
  - `prover/methods/guest/src/swiftsync.rs`.
- **OpenVM:**
  - `crates/circuits/mod-builder/src/{builder.rs, core_chip.rs, symbolic_expr.rs}`;
  - `crates/circuits/primitives/src/bigint/*`;
  - `extensions/algebra/{circuit/src/modular_chip/{is_eq,muldiv,addsub}.rs, moduli-macros/src/lib.rs, guest/src/{lib,exp_bytes}.rs}`;
  - `extensions/ecc/{circuit/src/weierstrass_chip/{add_ne,double}/mod.rs, guest/src/weierstrass.rs, sw-macros/src/lib.rs}`;
  - `extensions/sha2/{guest,circuit/src/sha2_chips/{main_chip,block_hasher_chip}/air.rs}`;
  - `crates/circuits/sha2-air/src/air.rs`;
  - `guest-libs/verify-stark/{guest,circuit/src/{lib,extension,verifier/air,output/air}.rs}`;
  - `extensions/deferral/circuit/src/{output,call}/air.rs`;
  - `crates/verify/src/{lib,deferral,pvs}.rs`;
  - `crates/sdk/src/prover/deferral/agg.rs`;
  - `audits/v2/v2.0.0-zksecurity-report.pdf` (findings #00-#26).
- **stark-backend:** `crates/stark-backend/src/verifier/batch_constraints.rs:358`.
