# C8 step 4 on hazync-coord: the engine-only host against engine/openvm's (28.11.2026)

/root/c8s4 on hazync-coord, 4 cores (taskset 12-15, nice), one hzengine (engine/host at 7620ec78), image keys-ch
(c7fe0eb85d2d4c10..., the board dry run's core image). Base = engine/openvm 7620ec78 (c8-step4's parent) built as the
board dry run builds it (RISC0_SKIP_BUILD=1: METHOD_ID zero); new = branch c8-step4. Scripts and their outputs here.

- setup.sh / setup.out: the checkouts and builds (base host 2,594 s: risc0's C++ kernels).
- export_cmp.sh / export.out: engine-export of prover/block_{130000,140000,741000,962000,965500}.json from both hosts
  (new at 199f4fde), every file compared: byte-identical but agg.in, agg_collect.in and method_id, which differ only in
  the image id at bytes 4..36 (base zero, new c7fe0eb8). EXPORT_CMP OK for all five.
- board_cmds.sh / board.out: the board dry run's host commands (new at b2dda1ea): prep 1..8 with leaf bundles (states
  and bundles byte-identical), libzync-genesis-tip, method-id, seg-po2, prove-range-bridge 1 and 2 and fold-range 1+2
  from both hosts (journals identical; proof files differ, as two STARKs of one statement do), verify-any --check-end
  (RANGE-OK ... end=board, the same from both) and its refusals (tampered journal and proof bytes, block 2 as range 1,
  no board key). The line "method-id with no engine set up ... exit 0" prints the exit of the echo, not of the host
  (the host exits 1: run by hand, and on Windows in run 46156577125).
