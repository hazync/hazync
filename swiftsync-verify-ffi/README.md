# `swiftsync-verify-ffi`: the libzync chain-proof verifier as a C library

One call, `swiftsync_verify(...)` (`include/swiftsync_verify.h`), running the same checks as the CLI and the WASM
build. For C or C++ hosts, e.g. a SwiftSync node in Bitcoin Core (`history/experimental/libzync-bench/retired-docs/CORE_HOOK.md` sketched where).

```sh
cargo build --release --manifest-path swiftsync-verify-ffi/Cargo.toml
cd swiftsync-verify-ffi && cc -O2 -Iinclude test.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test && ./test
```

`test.c` runs 18 cases through the C ABI: the vectors' engine proof of mainnet 1..200 under its image id and pinned
key (`DEMO`), the same proof as signet, against another image id, with no image id, with a key whose SHA-256 is not the
pin, with no key, five one-change copies (proof bit, journal bit, byte appended, cut, a proof length past the end), the
bundles (one `DEMO`, a header that lies, version 1), refusals, `swiftsync_verify` without a key (no engine image is
published yet: `FAIL` with the reason) and `swiftsync_image_id` (NULL until one is). `test_engine.c` checks a coin fact
on the same proof. Engine proofs only since the cutover (27.11.2026); RISC Zero receipts are checked with v0.27.1's
library. `test.out` is the result at this commit. The
result struct
returns the proven height, tip hash (internal order), chainwork (LE) and UTXO count, so the caller can do step 7
(the tip is on its own chain).
