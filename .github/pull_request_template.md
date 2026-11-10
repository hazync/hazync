## What changed


## Evidence
<!-- Commands run and their result. A test counts only if it can fail: say which positive control
     you ran, or why none applies. Measured numbers say what, on what, and what is still unmeasured. -->


## METHOD_ID impact
- [ ] Touches nothing the guest compiles: no change under `prover/methods/`, and none in a crate or
      `#[path]`-included file listed by `scripts/check-guest-inputs.sh` / `reproduce/METHOD_ID`
- [ ] Changes the guest. Even a comment moves the id. Say why this is worth a re-baseline and
      re-pin the published images (`reproduce/pin-libzync-ids.py`, which writes `reproduce/LIBZYNC_IDS`),
      then `reproduce/METHOD_ID`, `reproduce/GUEST_ELF_SHA256` and `scripts/lineage.sh --write`

## Docs
- [ ] Docs updated, or not needed because:
- [ ] Adds or changes a coordinator route, or an environment variable read by `coordinator/server.py`,
      `coordinator/hazync` or `coordinator/run-workers.sh`: ran `python3 scripts/gen-coordinator-reference.py`
      and committed `docs/COORDINATOR_REFERENCE.md` (CI fails on drift)
