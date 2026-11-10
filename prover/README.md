# Hazync prover

The zkVM prover for **libzync**: Bitcoin Core v28.0's own consensus code and libsecp256k1, compiled to RISC-V and
run inside RISC Zero, with the patches in [`../patches/`](../patches/). It proves a block valid as chunks plus an
aggregate, keeps coins in the coin log and a counting trie, and joins independently proved blocks into one proof.
What it proves and how: [`../docs/libzync/LIBZYNC.md`](../docs/libzync/LIBZYNC.md).

The host is **libzync-only**: it refuses to build without `HAZYNC_LIBZYNC=1`. The original per-signature CORE design
and its commands are in [`../history/`](../history/README.md); the guest still carries its CORE-mode branches until
the next deliberate image change, and a libzync image refuses those modes.

## Layout

```
prover/
├── host/            driver: builds witnesses and hints, proves, joins, verifies (target/<profile>/host)
├── methods/
│   ├── guest/       the zkVM program: Core's C++ wrapped by verify_input.cpp; main.rs drives chunks (mode 4),
│   │                the aggregate (5), the trie update (17) and joins (18)
│   └── build.rs     builds the guest, or embeds a prebuilt one checked against its pins
├── zyncapps/        zyncAPPs: separate guests that answer one question from a core proof
├── fetch_block*.py  fetch a block's JSON (explorer or archive node); how the block_*.json fixtures are made
├── tools/           block-file tools (mkblock.py) and analysis scripts
├── testdata/        libzync fixtures (libzync/), activation-boundary blocks, Core C++ checks (chainparams,
│                    retarget differential) and retarget vectors
└── block_*.json     real mainnet blocks, the HAZYNC_BLOCK input to the single-block commands
```

## Build

The published images are embedded, not rebuilt: a guest built anywhere but the reproducible container has a
different id. Take the ELF from `reproduce/libzync.Dockerfile` (or CI's `libzync-hosts` artefact) and pin it with a
directory holding its `METHOD_ID`, `GUEST_ELF_SHA256` and `PROFILE` (`reproduce/LIBZYNC_IDS` has the values;
`scripts/libzync/ci/build-hosts.sh` writes such a directory):

```bash
HAZYNC_LIBZYNC=1 HAZYNC_GUEST_ELF=<mainnet ELF> HAZYNC_GUEST_PIN_DIR=<pins> \
    cargo build --release -p host [--features cuda]      # add HAZYNC_NETWORK=signet for the signet image
```

To build the guest from source instead, `scripts/libzync-base.sh <dir>` prepares Core and libsecp with the patches,
then `HAZYNC_BASE=<dir> HAZYNC_LIBZYNC=1 cargo build --release -p host` (a different, unpublished id).

## Commands

```bash
H=./target/release/host
$H method-id                                              # the embedded image id
$H libzync-prep <lo> <hi> <json dir> genesis <out>        # the states and hint bundles for blocks lo..hi
$H libzync-prove-range <lo> <hi> <json dir> genesis <out> # prove lo..hi from genesis (chunks, trie, aggregate)
$H libzync-join <left.rcpt> <right.rcpt> <out.rcpt>       # join two adjacent proofs (mode 18)
$H libzync-verify <receipt> [--expect <h>:<hash>]         # OK / DEMO / FAIL, as swiftsync-verify
$H libzync-snark-wrap <receipt> <out>                     # Groth16 wrap of a chain receipt
HAZYNC_BLOCK=block_962000.json $H agg-exec-fake           # one block's aggregate in execute mode, no GPU
$H test-locks | test-merkle | core-vectors ...            # guest self-tests (modes 8, 9, 10)
```

On the original board coordinator a libzync host answers `prove-range-bridge`, `fold-range` (a join) and `verify-any`
(`host/src/lz_coord.rs`). `chunk-profile`, `prove-chunk`, `agg-chunks` and the `seg-*` commands split one block over
many cards. Every setting: [`../docs/OPERATIONS.md`](../docs/OPERATIONS.md) §6.1 (written for the CORE fleet; the
libzync settings are in `host/src/lz.rs` and `docs/libzync/`).

The libzync test pass is `scripts/libzync/ci/` (run by `.github/workflows/adversarial.yml`).
