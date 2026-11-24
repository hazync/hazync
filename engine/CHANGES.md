# What the engine changes relative to the product (RISC Zero) guest

The engine compiles the product guest's own source (`engine/guest/assemble.sh` copies it; the product tree is never
edited). Everything that differs is listed here, with the reason. Nothing here changes a consensus rule.

## Source edits (engine/guest/edits.py, each anchored to exactly one place)

| edit | why |
|---|---|
| `mod xver` -> a stub that panics | cross-version handover verifies OLD RISC Zero proofs; an engine chain starts at genesis and has none |
| `batch_ecdsa.rs` -> `batch_ecdsa_ovm.rs` | same public items, hint layout, transcript and batch equation; group arithmetic on OpenVM's secp256k1 extension instead of RISC Zero bigint2 circuits |
| `liftx_hint` inert when `liftx-hint` is off | RISC Zero's hinted key lift (patch 0013) is replaced by libsecp's sqrt hook |
| `env::finalize()` at the end of `main` | the journal's sha256 becomes the proof's 32 public-value bytes |
| `sys_sha_compress` defined in Rust | Core's and libzync's SHA call RISC Zero's syscall; here it runs on OpenVM's SHA-256 extension (byte swaps at the boundary: the RISC Zero layout; to be removed, see below) |
| `hz_fe_sqrt` defined in Rust | libsecp's sqrt hook: square root mod p on OpenVM's modular extension |
| `openvm::init!()` | OpenVM's moduli/curve setup |

## Platform (engine/platform)

`risc0_zkvm::guest::env` on OpenVM: the same input stream and journal bytes. `env::verify(image, journal)` is an
OpenVM deferred STARK verification of the proof named by the next control-chunk commitment; it requires
sha256(exe || vm) of that proof = `image` and its public values = sha256(journal). The image id the guest carries
(`self_id` in ChainState) is therefore sha256(app_exe_commit || app_vm_commit) instead of a RISC Zero image id.
`env::cycle_count` returns 0 (it only feeds TXCYC diagnostics).

## Consensus C/C++ (engine/guest/base.sh)

Core: exactly the product's tree (v28.0 + 0001 0002 0009 0010 libzync/0101).
libsecp: pristine v0.5.1 + `secp-0201-openvm-batch-liftx.patch` — the batch hooks of libzync/0102 at the same call
sites and kinds (field elements via libsecp's own `fe_get_b32`), and the sqrt hook. None of the product's RISC Zero
field/group patches (0005 0006 0008 0012 0013), and not 0103 (libsecp's SHA on the RISC Zero accelerator).

## Build

No fat LTO (product: fat). The bitcode linker refuses the duplicate `sys_sha_compress`; native SHA removes the symbol.

## Commitments

ChainState's `self_id` = the engine image id (above). Every other field means what it means in the product. The trie
journal, chunk journals and the rest of ChainState are byte-identical to RISC Zero's for the same input (checked per
block in `prove-block`).
