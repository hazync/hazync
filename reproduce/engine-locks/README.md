The engine guests' Cargo.lock files, pinned (reproduce/engine-ids.sh builds with OPENVM_BUILD_LOCKED=1, cargo --locked).
The assemble scripts (engine/guest/assemble.sh, engine/zapps/assemble_guest.sh, engine/zapps/headers/assemble.sh,
engine/handover/assemble.sh) copy the image's lock from here (since 27.11.2026; HZ_UNPINNED_LOCK=1 leaves it out), so a
native build uses the same crate versions as the container. Before, they wrote none: every build resolved crates.io
and an id could move with a crate release.

- `zeroconf.Cargo.lock`: the build box's lock behind keys-z-zeroconf3 (image 1ee5f2d7, commit 10d3c680, 27.11.2026).
- `core.Cargo.lock`: the build box's lock behind keys-ch (image c7fe0eb8, 26.11.2026) did not satisfy HEAD's manifest
  (7aca585c added the optional verify-core dependency, and a lock lists optional dependencies), so cargo extended it
  without --locked on 27.11.2026: every package of the old lock kept its version (the diff only adds packages, names
  them with their version where two versions now exist, and lists optional dependencies).
- `hints`, `utreexo`, `utreexo-tip` (keys-z-hints / -utreexo / -utreexo-tip, 26.11.2026), `headers` (keys-hdr29,
  26.11.2026) and `handover-verifier` (keys-hv, 26.11.2026): the build box's locks behind those images, as they were.
  `core-signet` uses `core`'s (a feature does not change the lock).
