# hzengine as a product component: first audit (ROADMAP 2.7, 27.11.2026)

What the product host relies on: `hzengine serve` (engine/host/src/serve.rs) and the host's client
(prover/host/src/engine_backend.rs). Then the CLI surface.

## Fixed in this pass

| finding | fix | checked |
|---|---|---|
| serve accepted any extra field: a misspelt one (`self-id`) silently became its default, here the wrong stream layout | `deny_unknown_fields` on the request | unit test: every request the host and block.rs send parses; two misspelt ones are refused ("unknown field") |
| serve's header listed three ops; it serves four (`prove_segments`) | the list completed, with the refusal rule and RUST_LOG | |
| a server that died (a panic, the OOM killer) showed in the host as `engine: bad reply "": EOF ...`, or as a failed `expect` | the host says the server is gone or ended without replying, with its exit status, and points at its stderr | a fake engine that prints its ready line and exits 3: `engine: the server is gone (exit status: 3); see its stderr above` |

## Open: for the user, or for the cutover

1. **RISC Zero-shaped names.** `r0.rs`, "a RISC Zero input stream" in `prove`'s help text, `--self-id` meaning "the product host's
   METHOD_ID bytes". The streams really are risc0's serde words, which the engine guests keep, so the names are not wrong
   today. They read as legacy once RISC Zero is removed: rename them at the cutover, in one change.
2. **The published proof file comes from `wasm-inputs`.** Its help still says "FEASIBILITY Q11 ... proof.zst". It writes
   proof.hze (HZE1), the file every verifier takes, plus vk.bin and vk.sha256. A product needs a plainly named command for
   this (e.g. `hzengine pack`). That is a CLI decision: scripts and docs call `wasm-inputs` today.
3. **Self-test commands in the product binary.** `selftest-leaf`, `selftest-node` and `selftest-negative` are M1 development
   checks. Keep them (cheap, useful on a new machine), or hide them behind a feature.
4. **Engine failures panic in the host** (exit 101 with the message), as the RISC Zero path does. A proving run cannot go on
   after either failure, so this is consistent. A long-lived service would want errors instead: Stage 3 (board, tip
   controller).
5. **Logs.** The SDK's progress spans are INFO on stderr and land in the host's log. RUST_LOG=warn quiets them (now in
   serve's header). The default stays INFO, because the timings in the results come from those spans.
