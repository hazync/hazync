# `swiftsync-verify-ffi`: the libzync chain-proof verifier as a C library

One call, `swiftsync_verify(...)` (`include/swiftsync_verify.h`), running the same checks as the CLI and the WASM
build. For C or C++ hosts, e.g. a SwiftSync node in Bitcoin Core (`history/experimental/libzync-bench/retired-docs/CORE_HOOK.md` sketched where).

```sh
cargo build --release --manifest-path swiftsync-verify-ffi/Cargo.toml
cd swiftsync-verify-ffi && cc -O2 -Iinclude test.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test && ./test
```

`test.c` runs 13 cases of the vectors through the C ABI, CI's `adversarial.yml` among them: the real mainnet 1..200
proof of the v0.24.0 image `8888d59c` (RISC Zero 3.1.0; `DEMO`), the same proof as signet, against v0.23.0's image
`863c9ce3`, and with a flipped bit (all `FAIL`), v0.23.0's RISC Zero 3.0.5 proof (`FAIL`: 3.1.0 refuses every 3.0.x
receipt), and refusals. The committed `test.out` predates the real-proof cases (8 refusals); rerun to refresh it. The
result struct
returns the proven height, tip hash (internal order), chainwork (LE) and UTXO count, so the caller can do step 7
(the tip is on its own chain).
