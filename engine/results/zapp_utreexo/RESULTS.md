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

To do: utreexo-tip on the engine (a base PROOF as its child), zeroconf (a speculative chunk proof), proofs on a GPU.
