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

1. ✅ **Done 27.11 (cutover C6): RISC Zero-shaped names.** `r0.rs` is `stream.rs`; `prove`'s help says "an input stream (the
   guests' word encoding)", and `--self-id`'s help names the core streams' `self_id` field instead of the product host's
   METHOD_ID. The flag and serve's `self_id` field keep their names: `self_id` is the journal field the engine keeps, and
   the scripts in results/ that reproduce each run use the flag. Before: `r0.rs`, "a RISC Zero input stream" in
   `prove`'s help text, `--self-id` meaning "the product host's METHOD_ID bytes".
2. ✅ **Done 27.11: `hzengine pack --dir --proof --out file.hze [--key key.bin]`.** It needs only agg.vk and baseline.json (0.17 s, against ~25 s for wasm-inputs, which loads sdk.pk). It checks the file with the readers' own verifier (verify-core) and writes nothing that fails; it prints the image id and the key's pin. Checked on the box: zeroconf's and hints' files and key byte-identical to wasm-inputs'; a changed journal and another image's keys refused, no file written. Before: Its help still says "FEASIBILITY Q11 ... proof.zst". It writes
   proof.hze (HZE1), the file every verifier takes, plus vk.bin and vk.sha256. A product needs a plainly named command for
   this (e.g. `hzengine pack`). That is a CLI decision: scripts and docs call `wasm-inputs` today.
3. **Self-test commands in the product binary.** `selftest-leaf`, `selftest-node` and `selftest-negative` are M1 development
   checks. Keep them (cheap, useful on a new machine), or hide them behind a feature.
4. **Engine failures panic in the host** (exit 101 with the message), as the RISC Zero path does. A proving run cannot go on
   after either failure, so this is consistent. A long-lived service would want errors instead: Stage 3 (board, tip
   controller).
   **Assessed 27.11 (no change):** `serve` already answers every error as `{"ok": false, "error": ...}` (serve.rs:
   `handle` returns Result); only a panic inside the SDK ends the process, and the host then reports "the server is
   gone (exit status ...)" (AUDIT item from the first round). The board runs the host once per job (prove-range-bridge,
   fold-range, verify-any: Stage 3.2), so a panic is a failed job the worker retries, as on RISC Zero. Catching panics
   inside `serve` would keep a process whose GPU state may be half torn down: not done. Re-check for the tip
   controller (3.1), the one long-lived caller.

5. **Logs.** The SDK's progress spans are INFO on stderr and land in the host's log. RUST_LOG=warn quiets them (now in
   serve's header). The default stays INFO, because the timings in the results come from those spans.
6. ✅ **Done 27.11: zeroconf's own kind (0x5A5A0001).** See engine/zapps/zeroconf/fixture/README.md.
