# Run your own coordinator

> This page describes running a **Hazync v2** (libzync engine) coordinator, as the public board at api.hazync.org
> has run since 2026-11-12. The guide for a Hazync v1 coordinator (archive bridge, witness bundles,
> `reproduce/METHOD_ID`) is kept in [`history/RUN_YOUR_OWN_COORDINATOR.md`](history/RUN_YOUR_OWN_COORDINATOR.md).

> **Two different things are called "coordinator" in this project.** This page is about the **board
> coordinator**: the long-lived service that hands out blocks, verifies submissions and keeps the board. It never
> proves and needs no GPU. The **segment coordinator** ([`OPERATIONS.md`](OPERATIONS.md)) splits one block's proof
> across machines; the two share no code.

A coordinator hands out proving work and keeps the board. It is **not a trust anchor**, and cannot forge a proof
(below): losing one loses the work queue and the scoreboard, never the validity of anything proved. Anyone can
stand one up and point provers at it.

The units below are the public board's, in [`../coordinator/deploy/libzync/`](../coordinator/deploy/libzync/). They
are written for one box (paths, the `hazync` user, ports); adapt them rather than copying them blind. How the public
board was switched to them is [`../coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md).

## What it is made of

There is no "bridge" in v2. Four pieces run on the coordinator box:

| piece | unit | what it does |
|---|---|---|
| block source | `libzync-board-fetch.service` | `fetch_block_rpc.py` pulls blocks from your `bitcoind` as `block_<h>.json`, a window ahead of prep |
| prep pass | `libzync-board-prep.service` | `libzync-prep-run.sh` runs `host libzync-prep`: the state before every block (coin log, counting trie, BIP30 tree), and one **leaf bundle** per block, `leaf_<h>.bin` |
| leaf pruner | `hazync-lz-prune-leaves.service` (+ `.timer`) | deletes leaf bundles of blocks proved below the frontier, so paced prep can go on |
| coordinator | `hazync-lz-coordinator.service` | `coordinator/server.py` with `HAZYNC_LIBZYNC=1`: claims, verification, joins, the genesis-anchored spine, the board |

Optional: `hazync-lz-zync-serve.service` (the zyncAPP fact service behind `/api/zync/*`, wired in by
`hazync-lz-coordinator.zync-serve.conf`), the offsite copies `hazync-lz-offsite-*`, and the checks
`hazync-lz-check-{proofs,spine}` and `libzync-xcheck`.

## The honest cost, first

The software is small. **The prep data is not**, and prep is slow.

- **A full `bitcoind`.** `prover/fetch_block_rpc.py` says it needs `txindex=1`, `prune=0` and Core 25 or newer
  (`getblock` verbosity 3, which returns each input's prevout).
- **Disk for the coin log and trie.** ESTIMATED, not measured for the whole chain
  ([`libzync/ROADMAP.md`](libzync/ROADMAP.md) H-B): a server with ~2.5-3 TB NVMe, 32 GB+ RAM, 8+ cores.
- **Time.** Prep is single-threaded. MEASURED on the board's box on 2026-11-08: about 0.9 blocks/s to ~367,000, then
  0.18-0.28 blocks/s once the trie (59 GB) outgrew RAM (62 GB), bound by the volume's write rate. Same source.

Prep is paced by proving: `HAZYNC_PREP_LEAF_WINDOW` caps the leaf bundles waiting to be proved, and the pass pauses
until the pruner removes proven ones. So a new coordinator does not need the whole chain prepared before it can serve
work, only the next window.

## The host binary

Every piece that runs a host needs the **v2 host**, `hazync-host-x86_64-linux-gnu` from release v0.24.0 (or a build
from `reproduce/libzync.Dockerfile`). Check it before anything else:

```sh
host method-id     # must print 8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4
```

A laptop build of the same source prints another id ([`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §8), and a
coordinator on another id is a different board: no public prover's proofs will be accepted by it. Prep stamps every
bundle with the image that prepared it, so prep and the coordinator must run the same image; v0.23.0's host
(`863c9ce3…`) is withdrawn.

The public board installs it twice: as `/opt/hazync-lz/bin/host` (the coordinator verifies with it) and as
`$LZ_ROOT/bin/host_lz` (what `libzync-prep-run.sh` runs).

## Prep and the block source

Both units read `LZ_ROOT` (the board uses `/srv/bulk/libzync-board`) and expect, under `$LZ_ROOT/bin/`, copies of
`prover/fetch_block_rpc.py`, `coordinator/deploy/libzync/libzync-prep-run.sh` and `host_lz`.

```ini
# libzync-board-fetch.service
Environment=LZ_ROOT=/srv/bulk/libzync-board LZ_HI=974936 LZ_WINDOW=1500 LZ_JOBS=2
Environment=HAZYNC_RPC_COOKIE=/var/lib/bitcoind/.cookie

# libzync-board-prep.service
Environment=LZ_ROOT=/srv/bulk/libzync-board LZ_HI=974936
Environment=HAZYNC_PREP_SAVE_EVERY=1000 HAZYNC_PREP_REPORT_EVERY=10000 HAZYNC_PREP_MIN_FREE_GB=250
Environment=HAZYNC_PREP_LEAF_WINDOW=5000
```

- `LZ_HI` is the last height of the run. `fetch_block_rpc.py` also reads `HAZYNC_RPC_HOST`, `HAZYNC_RPC_PORT`,
  `HAZYNC_RPC_AUTH` or `HAZYNC_RPC_CLI` if your node is not local.
- `libzync-prep-run.sh` creates `blocks/ cl/ trie/ out/ leaves/` under `LZ_ROOT`, sets `HAZYNC_COINLOG_DIR`,
  `HAZYNC_TRIE_DIR`, `HAZYNC_PREP_CONSUME=1` (block files are deleted once used) and `HAZYNC_PREP_LEAF_BUNDLES`, and
  resumes by itself from the newest complete save point in `out/` (from genesis if there is none).
- The fetcher picks its start height once, at startup, from prep's save points (`--after-saves`). If you restart
  prep from scratch, restart the fetcher too, or prep waits for `block_1.json` for ever (SWITCH.md step 0).

## The coordinator

`hazync-lz-coordinator.service`, as the board ran it before the switch (the switch moved it to port 8899 and added
the public board's drop-ins; SWITCH.md step 3):

```ini
User=hazync
WorkingDirectory=/opt/hazync-lz/coordinator
Environment=HAZYNC_LIBZYNC=1
Environment=COORD_PORT=8900
Environment=COORD_BIND=127.0.0.1
Environment=COORD_DB=/var/lib/hazync-lz/coordinator.db
Environment=COORD_STATE=/var/lib/hazync-lz/coord_state
Environment=COORD_PROOFS=/var/lib/hazync-lz/proofs
Environment=COORD_SPINE=/var/lib/hazync-lz/spine
Environment=WITNESS_DIR=/var/lib/hazync-lz/witnesses
Environment=HAZYNC_BRIDGE_OUT=/srv/bulk/libzync-board/leaves
Environment=HAZYNC_HOST=/opt/hazync-lz/bin/host
Environment=GENESIS_TIP=852c5c7e009317c2dd232dcf9b1a9e2babd58df324add461a6ff61824f390167
Environment=VERIFY_MODE=real
Environment=SEED_RANGES=0
ExecStart=/usr/bin/python3 /opt/hazync-lz/coordinator/server.py
```

plus the drop-in `hazync-lz-coordinator.state-dir.conf`:

```ini
Environment=HAZYNC_LZ_STATE_DIR=/var/lib/libzync-board/out
```

What each one is for:

- **`HAZYNC_LIBZYNC=1`** makes it a v2 coordinator: it serves `leaf_<h>.bin` from `HAZYNC_BRIDGE_OUT` (the name is
  kept from v1; it is prep's leaf directory) and tells the host the claimed start height when it verifies.
- **`HAZYNC_HOST`** is the path to the v2 host binary, not a node address. Empty means `VERIFY_MODE=mock`: every
  proof accepted unverified.
- **`GENESIS_TIP`** is the genesis link digest, printed by `host libzync-genesis-tip`; for mainnet it is the pinned
  genesis anchor. The default in `server.py` is v1's genesis hash, so a v2 coordinator must set it.
- **`HAZYNC_LZ_STATE_DIR`** is prep's `out/`: once a leaf bundle is pruned, the host finds the state before a range
  from prep's `state_<h>.json` there, so stored proofs stay re-verifiable (`prover/host/src/lz_coord.rs`).
- **`SEED_RANGES=0`**: by default (60) an empty database is seeded with 60 ranges of `RANGE_SIZE`
  blocks from height 0; the libzync unit turns that off.
- `/var/lib/hazync-lz` must exist and be writable by the service user, or SQLite cannot create the database.

Other settings the public board relies on, with their defaults in `server.py`: `RANGE_SIZE` (1000), `CLAIM_TTL`
(3600 s without a heartbeat before a claim is reoffered), `CLAIM_GRACE` (600 s for a claim that never beat),
`CLAIM_OPEN_MAX` (4 open claims per key: one key running more workers than that starves itself), `TRUSTED_PROXIES`
(`127.0.0.1,::1`). Every variable, with its default, is in [`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md),
generated from the source.

**It refuses to bind a public interface in an insecure mode**: `VERIFY_MODE` other than `real`, no ed25519 library
(`python3-cryptography`), or `COORD_ALLOW_MOCK` / `COORD_ALLOW_UNSIGNED` set. Bind `127.0.0.1` behind a reverse
proxy, or fix the config; `COORD_ALLOW_PUBLIC_INSECURE=1` overrides, not for a real board.

Check it is up and on the right image:

```sh
curl -s 127.0.0.1:8900/api/meta     # "libzync": true and method_id 8888d59c…
```

### The leaf pruner

```ini
# hazync-lz-prune-leaves.service
Environment=LEAVES=/srv/bulk/libzync-board/leaves COORD=http://127.0.0.1:8900 MARGIN=1000
ExecStart=/usr/local/sbin/hazync-lz-prune-leaves --apply
```

`/usr/local/sbin/hazync-lz-prune-leaves` is `coordinator/deploy/libzync/hazync-lz-prune-leaves.sh`. It deletes
`leaf_<n>.bin` once block n is at least `MARGIN` blocks below the coordinator's frontier, and keeps the heights in
`KEEP` (default `1`) for good: block 1's bundle is what the worker's `selftest` and the sponsor bot's GPU smoke prove.
Without the pruner, prep stops at `HAZYNC_PREP_LEAF_WINDOW` bundles and the board runs dry.

## Pointing workers at it

Workers find the coordinator through `COORD_URL` (default `https://api.hazync.org`):

```sh
export COORD_URL=https://coord.example     # or http://127.0.0.1:8900 on the same box
./hazync selftest                          # host present, its image id matches your /api/meta, can verify a real proof
HAZYNC_HOST=/path/to/hazync-host-x86_64-linux-gnu-cuda ./hazync-run-workers.sh 4
```

`hazync-run-workers.sh` (`coordinator/run-workers.sh`) refuses to start when the host's image id differs from the
coordinator's `/api/meta`, and smoke-tests the GPU on your `/api/witness/1`. `MODE=fold` or `MODE=mixed` adds
workers that join proofs; on a v2 coordinator the spine also advances by joins (`fold-range`), since a v2 host has
no `extend-spine`. A worker needs no node and no chain data: only the leaf bundles you serve. Setting up a worker in
full is [`../CONTRIBUTING.md`](../CONTRIBUTING.md).

## Why it cannot forge a proof

- **A receipt verifies against the image id, whoever coordinated it.** Users check the board's proofs with the
  verifier (`swiftsync-verify`, released as `hazync-verify-*`), against the published id and the pinned genesis
  anchor ([`SOUNDNESS.md`](SOUNDNESS.md)). A coordinator has no key or input that changes that answer.
- **The coordinator only verifies.** It runs `host verify-any <receipt> <lo>`; it never proves. A range starting at
  block 1 is checked against genesis; a later one against the state in its leaf bundle, but ranges only chain, and
  only join, when each starts exactly where the previous ends, and the spine is anchored at genesis. A wrong prepared
  state can only make a join fail, never forge (`prover/host/src/lz_coord.rs`, [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md)
  §3).
- **Prep is untrusted too.** A wrong bundle costs proving time, not validity.

What a coordinator *can* do badly is waste work: hand out blocks it cannot serve, or prune a bundle too early. It
also controls what its workers download, which is why the worker's handling of coordinator responses is on the
review list ([`EXTERNAL_REVIEW.md`](EXTERNAL_REVIEW.md) item 9).

## Not covered here

- **Peering between coordinators.** `server.py` still has v1's `PEER_COORDINATORS` (skip heights a peer has proved
  or is proving, adopt verified proofs from peers), described in the v1 guide. It has not been documented or checked
  for v2 here.
- **Following the tip.** A v2 board does not follow new blocks yet.
- **Payments, sponsorships and the sponsor bot**: [`COORDINATOR.md`](COORDINATOR.md).

## Related

- [`COORDINATOR.md`](COORDINATOR.md), and the generated [`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md)
- [`../coordinator/deploy/libzync/SWITCH.md`](../coordinator/deploy/libzync/SWITCH.md): how the public board was moved to v2
- [`libzync/LIBZYNC.md`](libzync/LIBZYNC.md) §3 (joins and prep), [`libzync/PROOF_BUNDLE.md`](libzync/PROOF_BUNDLE.md)
