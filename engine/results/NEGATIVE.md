# Negative controls (24.11.2026, build box, execute only: each must be refused inside the guest)

| control | how | result |
|---|---|---|
| child journal tampered | `hzengine selftest-negative --tamper journal` (one byte of a child's journal flipped; host pre-checks skipped) | refused: `recursion.rs:40` (public values != sha256(journal)) |
| child of another image | `--tamper image` (the node told a different exe commitment) | refused: `recursion.rs:37` (exe commit) |
| a dry run that would commit | `hzengine execute --dry` on block 976,379 chunk 0 (keys-hz3) | refused: `platform/risc0-zkvm lib.rs:171` "a dry run cannot commit a journal" |

The same checks guard the engine guest: `platform/risc0-zkvm` env::verify (image id = sha256(exe||vm), public values
= sha256(journal)), and finalize's refusal of a dry run.
