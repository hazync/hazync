# Hazync prover

The host for **Hazync v2** (the libzync design) on the OpenVM engine: it prepares blocks, writes the guest's input
streams, keeps the coin log, the counting trie and the BIP30 tree, and drives the engine (`hzengine`,
[`../engine/ENGINE.md`](../engine/ENGINE.md)) for every guest execution and proof. The guest is Bitcoin Core v28.0's own
consensus code and libsecp256k1 compiled to RISC-V, with the patches in [`../patches/`](../patches/); it proves a block
valid as chunks plus an aggregate and joins independently proved blocks into one proof. What it proves and how:
[`../docs/libzync/LIBZYNC.md`](../docs/libzync/LIBZYNC.md).

Since CUTOVER C8 (28.11.2026, [`../engine/CUTOVER.md`](../engine/CUTOVER.md)) the host has no RISC Zero backend: no
embedded guest, no RISC Zero prover, receipts or segment commands. v0.27.1 is the last release on RISC Zero; its proofs
stay checkable with that release's verifiers. The host is **libzync-only**: it refuses to build without
`HAZYNC_LIBZYNC=1`. Hazync v1, the original per-signature CORE design (last released as v0.22.1), is in
[`../history/`](../history/README.md).

## Layout

```
prover/
├── host/            driver: prep, witnesses and hints, the engine's streams, joins, the board's commands
│                    (target/<profile>/host); engine_backend.rs talks to `hzengine serve`
├── methods/
│   ├── guest/       the guest's SOURCES: Core's C++ wrapped by verify_input.cpp; main.rs drives chunks (mode 4),
│   │                the aggregate (5), the trie update (17) and joins (18). The engine assembles and builds them
│   │                (engine/guest/assemble.sh, hzengine build); nothing here builds them
│   └── build.rs     the guest's profile constants for the host (libzync/src/profile.rs): no zkVM, no guest build
├── zyncapps/        zyncAPPs: the guests' sources and the host that writes their engine streams (`--stream`)
├── fetch_block*.py  fetch a block's JSON (explorer or archive node); how the block_*.json fixtures are made
├── tools/           block-file tools (mkblock.py) and analysis scripts
├── testdata/        libzync fixtures (libzync/), activation-boundary blocks, Core C++ checks (chainparams,
│                    retarget differential) and retarget vectors
└── block_*.json     real mainnet blocks, the HAZYNC_BLOCK input to the single-block commands
```

## Build

```bash
HAZYNC_LIBZYNC=1 cargo build --release -p host          # add HAZYNC_NETWORK=signet for a signet host
```

A plain Rust build (Linux, macOS, Windows: `.github/workflows/windows-prove.yml` builds and checks the Windows one).
The image it works with is the engine's: `HAZYNC_ENGINE=<hzengine> HAZYNC_ENGINE_KEYS=<the image's key dir>` for every
command that runs the guest, `HAZYNC_ENGINE_VK=<key.bin> HAZYNC_ENGINE_VK_PIN=<its SHA-256>` for a board's verify-any.
Where a stream or a state names this host's image (bytes 4..36 of an aggregate, a join or a trie combine) it is that
engine image (`engine_backend::image_id`), never a constant.

## Commands

```bash
H=./target/release/host        # with HAZYNC_ENGINE + HAZYNC_ENGINE_KEYS for everything that runs the guest
$H method-id                                              # the engine image id (keys dir or board key); none: exit 1
$H libzync-prep <lo> <hi> <json dir> genesis <out>        # the states and leaf bundles for blocks lo..hi (no engine needed)
$H libzync-prove-range <lo> <hi> <json dir> genesis <out> # prove lo..hi from genesis on the engine (chunks, trie, aggregate)
$H libzync-join <left> <right> <out>                      # join two adjacent proofs (mode 18)
$H libzync-prep-rewind <out dir> <block dir> [--allow-deep]  # undo prep's stores back to a fork (docs/libzync/REORG.md §16)
$H libzync-verify <proof> [--expect <h>:<hash>]           # OK / DEMO / FAIL, as swiftsync-verify
$H libzync-hints <out>                                    # one block's hint bundle (HAZYNC_BLOCK), from engine runs
HAZYNC_BLOCK=block_962000.json $H engine-export <dir>     # one block's streams and journals for hzengine's tools
$H exec-chunk <i> | test-locks | test-merkle | core-vectors ...  # one chunk, guest self-tests (modes 8, 9, 10) on the engine
```

For the board coordinator (`HAZYNC_LIBZYNC=1`, [`../docs/COORDINATOR.md`](../docs/COORDINATOR.md) §1a,
[`../engine/BOARD_ENGINE.md`](../engine/BOARD_ENGINE.md)) the host answers `prove-range-bridge` (one block from its
`leaf_<n>.bin`), `fold-range` (a join, also how the spine advances), `verify-any <proof> <lo> [--check-end]`,
`libzync-genesis-tip`, `libzync-lineage` and `seg-po2` (`engine`: the engine sizes its own segments)
(`host/src/lz_coord.rs`). RISC Zero's `chunk-profile`, `prove-chunk`, `agg-chunks`, `agg-exec-fake`, the `seg-*`
commands, `libzync-snark-wrap`, `libzync-handover` and the receipt tools went with C8 (engine/C8_PLAN.md).

The libzync test pass is `scripts/libzync/ci/` (run by `.github/workflows/adversarial.yml`); its RISC Zero-shaped
jobs are being moved to the engine (engine/CI_RETURN.md).
