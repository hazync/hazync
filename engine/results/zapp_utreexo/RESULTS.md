# zAPPs on the engine: utreexo (Stage 2.3), and the zAPP hosts' single-source inputs (26.11.2026)

Commit 10d6112a: every zAPP host subcommand (hints, utreexo, utreexo-tip, zeroconf) builds its guest input ONCE, as a
word vector given both to RISC Zero's ExecutorEnv (`write_slice`) and to `--stream` for the engine. RISC Zero's
`write(&T)` is `write_slice(&serde::to_vec(T))` (risc0-zkvm 3.1.0 env.rs:316), so the RISC Zero input is unchanged by
construction; checked below. `hzengine prove/execute --self-id-at <byte>`: the engine writes its image id where a zAPP
stream keeps the guest's own id (utreexo and utreexo-tip: byte 0; the product host's streams stay at 4).

Build box, the engine chain 1..200 (core200: the engine's proof of block 200 and its state JSON; mainnet blocks 1..200).
zt.sh / zt.out, plus the same-ELF re-runs below.

## RISC Zero path unchanged (old host = 12:41 build, before the change; new = 10d6112a; SAME guest ELF via --elf)

| run (RISC Zero execute) | journal old vs new | statement + cycle lines |
|---|---|---|
| hints --stream to 200 | stream bytes IDENTICAL (69,232) | |
| utreexo base to 100 | IDENTICAL | same |
| utreexo base to 200 | IDENTICAL | same (11,900,987 user cycles) |
| utreexo tip step 100 -> 200 (--base-id cd8ea369...) | IDENTICAL | same (7,223,662 user cycles) |

(The old host was built with RISC0_SKIP_BUILD, so its own embedded guests and default --base-id are placeholders: the
first attempt without --elf / --base-id failed on the OLD host for that reason, zt.out.)

## utreexo base on the engine (keys-z-utreexo)

`zyncapp utreexo-prove ... 200 --stream` (69,268 bytes) -> `hzengine execute --self-id-at 0 --child block_200.ovm`
(HZ_CHILD_KEYS=keys-ch, the core proof verified in-guest): **EXECUTE_OK, cost 951,143,709; journal 1,400 bytes, identical
to RISC Zero's except bytes 4..36 = the statement's self_id** (each prover's own image id; everything else: height,
block, leaves 209, roots, core id, lineage, ... equal).

## utreexo tip step on the engine (keys-z-utreexo-tip; zt2.sh / zt2.out, commit f749cda3)

A tip step verifies TWO proofs of two different images: the previous utreexo statement's and the core's. hzengine now
takes HZ_CHILD_KEYS as a list (one keys dir per child): each child's deferral result is checked under its own image's
baseline (OpenVM's get_raw_deferral_results works proof by proof; the baseline enters only its host-side check that the
proof is of that executable; the STARK is verified under the VM's key), and the deferral state is built from those
results with OpenVM's generate_deferral_results, which is what get_deferral_state does for a single image.

1. Single-image regression: the utreexo base to 200 re-executed after the change: journal identical.
2. The utreexo base to 100 PROVED on the engine (CPU, 462 s); `hzengine verify` OK. Its image id
   340f6675... (sha256(exe || vm) from image.json) = the self_id in its journal.
3. `zyncapp utreexo-tip-prove ... base100.journal 200 ... --stream --base-id 340f6675...`, then
   `HZ_CHILD_KEYS=keys-z-utreexo,keys-ch hzengine execute --self-id-at 0 --child base100 --child block_200.ovm`:
   **EXECUTE_OK, cost 405,192,880; journal 1,328 bytes, identical to RISC Zero's tip step 100 -> 200 except bytes 4..68
   = the statement's self_id and base_id** (each prover's own two images).
4. Negative controls: the two images swapped, and one image for both: **refused** (`Invalid program commit`) before
   the guest runs.

To do: the tip step PROVED (and a step on a step), zeroconf (a speculative chunk proof as its second child), GPU timings.
