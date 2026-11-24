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
| coin log (`libzync/src/coinlog.rs`) `compress_w` on `zkvm_sha256_impl` | OpenVM's SHA-256 in its native state layout: IVs as they are (no pre-swapped `IV_*_W`), one swap of the output into the digest's bytes; RISC Zero's `sys_sha_compress` is not defined anywhere in the engine |
| `hz_fe_sqrt` defined in Rust | libsecp's sqrt hook: square root mod p on OpenVM's modular extension |
| `openvm::init!()` | OpenVM's moduli/curve setup |

## Platform (engine/platform)

`risc0_zkvm::guest::env` on OpenVM: the same input stream and journal bytes. `env::verify(image, journal)` is an
OpenVM deferred STARK verification of the proof named by the next control-chunk commitment; it requires
sha256(exe || vm) of that proof = `image` and its public values = sha256(journal). The image id the guest carries
(`self_id` in ChainState) is therefore sha256(app_exe_commit || app_vm_commit) instead of a RISC Zero image id.
`env::cycle_count` returns 0 (it only feeds TXCYC diagnostics).

## Consensus C/C++ (engine/guest/base.sh)

Core: the product's tree (v28.0 + 0001 0002 0009 0010 libzync/0101) + `core-0201-sha-openvm-native.patch`: Transform,
`HAZYNC_D64_STREAM`'s hz_d64 and `HAZYNC_SHA_STREAM`'s CSHA256 call OpenVM's `zkvm_sha256_impl` in Core's own state
layout instead of RISC Zero's byte-swapped `sys_sha_compress` (same digests; unaligned blocks staged in a local block).
libsecp: pristine v0.5.1 + `secp-0201-openvm-batch-liftx.patch` + `secp-0202-sha-openvm-native.patch` (libsecp's SHA-256
compression on OpenVM's extension, `HAZYNC_SHA_OVM`) — the batch hooks of libzync/0102 at the same call
sites and kinds (field elements via libsecp's own `fe_get_b32`), and the sqrt hook. None of the product's RISC Zero
field/group patches (0005 0006 0008 0012 0013), and not 0103 (libsecp's SHA on the RISC Zero accelerator).

## Build

Fat LTO + codegen-units=1, as the product.

## Commitments

ChainState's `self_id` = the engine image id (above). Every other field means what it means in the product. The trie
journal, chunk journals and the rest of ChainState are byte-identical to RISC Zero's for the same input (checked per
block in `prove-block`).
