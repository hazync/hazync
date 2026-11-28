# Bringing engine/openvm back to CI (plan, 28.11.2026)

The user, 28.11.2026: no CI has ever run on this branch (CI runs on pull requests and on main; the branch only ever ran
engine-windows.yml, through its request file). Against main's merge-base 874454fc the branch is 402 commits, 915 files,
+106,782 / -36,646, and adversarial.yml is still RISC Zero-shaped. Bringing it back is planned, not tried: local first,
one CI run at the end.

1. **Local CI inventory** (done 28.11.2026: engine/results/ci_local/RESULTS.md). Every step of every workflow that runs
   on push / pull_request, run here where it can run, from a detached tree of the branch head (not the working tree).
2. **The workflows made engine-shaped** (C8.5, the C8 agent's step 5): the RISC Zero heavy jobs (the libzync.Dockerfile
   container, its hosts, chain / blocks / vectors / rig / signet, the soundness suite) replaced by their engine
   equivalents (the engine container reproduce/engine.Dockerfile, the engine Core-vectors runner and rig of 818d978b,
   on CPU and in budget); the RISC Zero-shaped light steps listed in RESULTS.md moved to the engine; the real failures
   fixed (RESULTS.md, "real").
3. **The local pass repeated until green**, after the round-3 fixes and C8 step 4 land, with the same runner
   (engine/results/ci_local/runci.py), from a fresh detached tree each time.
4. **Right before merging: the main-revert check.** List main's commits since the merge-base (0 today); trial-merge in a
   scratch worktree; for each main commit, diff its paths between main and the merge result and confirm every change
   survives. Reason: an agent once committed with a stale tree and reverted merged work while CI stayed green.
   `git diff --stat origin/main...engine/openvm` is read by a person, not trusted to a green tick.
5. **Then one draft pull request**, CI once, every job read per job (a run's overall colour is not enough).
