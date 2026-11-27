# Round 1: the guest and build-side fixes (27.11.2026)

Findings from ASSESSMENT.md, each assessed (cause, every path, risk) before the change, and checked after it.

## R1-1 #1: a metering image could pass as a production image's proof

- Cause: `bench-noverify` (env::verify accepts without proofs) was kept out of releases only by the image id; finalize
  revealed sha256(journal) as in production, so a published bench image would have accepted forged children.
- Change: engine/platform/risc0-zkvm finalize reveals sha256(BENCH_TAG || journal) in a bench build
  (BENCH_TAG = "hazync/bench-noverify/v1"); `hzengine build` writes BENCH_NOVERIFY into a keys dir built with the
  feature (and removes it on a production rebuild); the host's public-value checks follow the marker (engine.rs
  pvs_of, stream.rs execute); `pack`, `wasm-inputs` and serve's `pack` (the files readers get) refuse a marked dir.
  A bench proof's public values are not sha256 of any journal, so verify-core refuses it too.
- Checked (laptop, the headers guest, headers 0..4,031): production image (exe 639aed3f) and bench image (exe b4dc6b69):
  both execute, journals identical (1,228 bytes, sha256 d1db000b..., = the recorded p0_4031.journal); the bench image
  with its marker REMOVED is refused ("journal does not match the public values"), i.e. its public values are not
  sha256(journal); `pack` and `wasm-inputs` on the bench dir are refused before reading anything.

## R1-5 #1: handover key provenance unchecked at build time

- Cause: assemble.sh embedded any key file named by its own hash; the guest checks image_id_of(key) == image, which
  binds only the key's baseline (a forged mvk can carry the genuine baseline). HANDOVER.md claimed a build-time check
  that did not exist.
- Change: engine/guest/check_xver_provenance.py, run by assemble.sh after the keys are embedded: for every lineage file
  whose last handover crosses OpenVM versions, a published record (reproduce/engine-records/<image>.json) must name that
  image with exactly the lineage's pin, the key must be carried, and a "test" record (a rehearsal's predecessor) is
  accepted only for lineage/test.rs; an ovm-xver assembly with no crossing lineage is refused. One test record: step 3's
  v2.0.2 predecessor (37adf7e2, pin e2519a33). HANDOVER.md corrected.
- Checked: the step-3 lineage with its real key passes; refused: the key not carried, another pin in the lineage, the
  test record used by lineage/mainnet.rs, no record, no crossing lineage (5 negatives, exact messages in the commit).
- Not done: a pinned lock for ovm-xver builds (its manifest gains the patched OpenVM tree's [patch] sections, so the
  lock must be resolved against that tree; left for the image rebuild at the freeze).

## R1-5 #3: in-guest handover negatives

- engine/results/handover_step2/hv_neg_r1.sh: case 4 (a genuine predecessor proof at another height) as a runtime
  negative in step 2's harness, and cases 5-8 (key != pin, another image's genuine key, lineage mismatch, non-chain
  journal) documented as purpose-built images. NOT RUN: it needs hazync-coord's CPU and step 2's artefacts, and keys-xv
  must be rebuilt after the round-1 guest fixes.

## R1-2 #1, #3, R1-4 A4

- edits.py edit 4: `fn main() {` asserted unique, and the brace it inserts before asserted to END main (what follows is
  a new item, a blank line or the end); a column-0 `}` inside main is refused (checked on a synthetic file). The edited
  main.rs is byte-identical to the old script's output.
- coinlog_ovm.py's comment (Rust's &mut cannot alias the block) and cshims.c's bad_alloc comment (EvalScript's catch
  turns it into a failed script) corrected, on the same lines (no panic location moves).

## R1-4 A1: the C heap and latent use-after-free

- Inspection: cached_tx's references live only within one C call (std::map nodes are stable across inserts;
  hz_cache_reset is called from Rust between transactions only); the legacy-sighash cache stores pointers as keys only,
  guarded by hz_tx_generation and hz_lsh_key (cleared by the guest's free). No use-after-free found.
- A generation tag for the double-free check: not implemented. The stale alias of a recycled block IS the new block's
  pointer, and a C pointer carries no tag, so a second free of it cannot be told from a legitimate free.
- Heap-poison run (laptop): the core guest built with `heap-poison` (blocks poisoned on free and on reuse; exe 5e8b9513),
  every chunk of the nine edge blocks executed: @@POISON@@

## R1-4 B3: producers and the node format

- zync-path, zync-serve, swiftsync-verify make_trie_fact and zapp-liveness `prove` set node format 2 explicitly; the
  statement zAPP's draft takes its proof's format (use_format_of(cs.profile)). zapp-liveness `make_liveness` stays
  caller-driven (documented): its test of an old chain sets format 1 on purpose, and forcing 2 there broke that test
  (measured: undo_leaf_openings_on_host_data failed, passes as documented). swiftsync-verify 22 + 8 passed, liveness 13 +
  4 + 1, statement 20 + 1.
- Found on the way: zapps/liveness/tests/engine.rs reads vectors/engine/*.hex, which .gitignore (`*.hex`) keeps out of
  the repo: on a fresh clone that test fails (it passed here only after copying them from the parent checkout).

## R1-3 #4 and the parameters' wording

- engine/ENGINE.md: "a chunk's all_valid = false means not proven, never invalid"; checked: a false all_valid fails the
  aggregate's asserts (no proof), and nothing reads it as proof of invalidity (the host only prints it).
- engine/results/security_params/RESULTS.md: one decimal (100.0 = 99.95-100.05), per circuit not the recursive
  composition, the app VM exactly at the height limit.

## Image ids

The platform (finalize) and comment changes move the core image: @@IDS@@
