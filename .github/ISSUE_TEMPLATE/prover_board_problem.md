---
name: Board or prover problem
about: A worker that fails, stalls or is rejected, or a board that looks wrong
---

<!-- Most rejections are a guest id that differs from the coordinator's: the board takes only image
     1ff5a129 (release v0.26.0) from block 2000, and 8888d59c (v0.24.0, v0.25.0) below it, and a run-workers.sh loop started on an older release must be
     restarted. Run `hazync-worker selftest` first and include its output. CONTRIBUTING.md
     ("If something breaks", "Several workers on one key") covers the common cases. -->

**What happened**


**Versions**
- `hazync-worker` release (`grep '^VERSION' hazync-worker`):
- Your guest id (`hazync-host-x86_64-linux-gnu-cuda method-id`, the CPU host, or `host.exe method-id` on Windows):
- Coordinator's guest id (`curl -s https://api.hazync.org/api/meta`):
- Segment size (`<host> seg-po2`, and `HAZYNC_SEG_PO2` if you set it):

**Machine**
- OS (Linux, Windows native, Windows via WSL2):
- GPU (`nvidia-smi -L`) and driver, or CPU only:
- RAM, and provider if rented:
- Launcher: `hazync-run-workers.sh` with `MODE=`, `hazync-worker` directly, or the Windows GUI:
- Workers claiming on this key, across every box that shares it (the board allows 4 open claims per key):
- Restarted or killed workers in the last hour? (their claims stay open until they lapse):

**Work**
- Block or range (for example `39413` or `1-2`):
- Handle shown on the board:
- Approximate time (UTC):

**Alert** (if `hazync-worker notify` pushed one: its title and text)

```
```

**`hazync-worker selftest` output**

```
```

**Worker log** (the lines around the failure; `hazync-run-workers.sh` writes `worker_<i>.log` under `LOG_DIR`, default `~/hazync-workers`)

```
```
