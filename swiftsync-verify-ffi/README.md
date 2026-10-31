# `swiftsync-verify-ffi`: the SwiftSync hints-proof verifier as a C library

One call, `swiftsync_verify(...)` (`include/swiftsync_verify.h`), running the same checks as the CLI and the WASM
build. For C or C++ hosts, e.g. a SwiftSync node in Bitcoin Core (`docs/libzync/CORE_HOOK.md` sketches where).

```sh
cargo build --release --manifest-path swiftsync-verify-ffi/Cargo.toml
cd swiftsync-verify-ffi && cc -O2 -Iinclude test.c target/release/libswiftsync_verify.a -lpthread -ldl -lm -o test && ./test
```

`test.out`: 9/9 cases on the real round-4 signet receipt and corrupted inputs, through the C ABI. The result struct
returns the proven height, tip hash (internal order), chainwork (LE) and UTXO count, so the caller can do step 7
(the tip is on its own chain).
