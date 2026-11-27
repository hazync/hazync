The engine guests' Cargo.lock files, pinned (reproduce/engine-ids.sh builds with OPENVM_BUILD_LOCKED=1, cargo --locked).
engine/guest/assemble.sh and engine/zapps/assemble_guest.sh write no lock, so an unpinned build resolves crates.io at
build time and an id could move with a new crate release.

- `zeroconf.Cargo.lock`: the build box's lock behind keys-z-zeroconf3 (image 1ee5f2d7, commit 10d3c680, 27.11.2026).
- `core.Cargo.lock`: the build box's lock behind keys-ch (image c7fe0eb8, 26.11.2026) did not satisfy HEAD's manifest
  (7aca585c added the optional verify-core dependency, and a lock lists optional dependencies), so cargo extended it
  without --locked on 27.11.2026: every package of the old lock kept its version (the diff only adds packages, names
  them with their version where two versions now exist, and lists optional dependencies).
