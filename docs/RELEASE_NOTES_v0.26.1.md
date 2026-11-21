> [!WARNING]
> **Do not use v0.26.1. Use [v0.27.1](https://github.com/hazync/hazync/releases/tag/v0.27.1) instead.**
> Its Linux CUDA host (`hazync-host-x86_64-linux-gnu-cuda`) stops on its first GPU step, and its Linux verifiers need glibc 2.39, so they do not start on Ubuntu 22.04 or Debian 12. v0.27.1 fixes both, with the same mainnet image (`46666b6f`): nothing needs re-proving. The notes below are kept as they were.

**The Windows app proves on more machines: older NVIDIA cards, and any PC's processor.** v0.26.1 is a host, app and
coordinator release. The image does not change: mainnet stays **`46666b6f`**, and the board carries on from where it is.

```
mainnet  46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c   (unchanged from v0.26.0)
```

## The Windows app

- **A CPU prover ships in the installer** (`hazync-host-x86_64-windows-cpu.zip`, a new release asset, 14 in all).
  - Who it's for: a PC with no NVIDIA card, or a card the GPU prover doesn't support, can still prove.
  - What the app says: it offers the processor and states what it costs. The one measurement so far: mainnet block 1 in
    1,433 s (about 24 min) on a 4-vCPU Windows runner. Bigger blocks take longer.
  - How it's built: with the C runtime linked statically. The release gate refuses any import that isn't part of
    Windows.
- **Older NVIDIA cards (Pascal, e.g. the GTX 1050 Ti)** can prove, behind the opt-in "Try my GPU anyway" (#739, #727).
  - The cause: the prover's lookup tables used system-scope atomics. The Windows driver refuses to load a module
    containing those on cards without host-native atomics. They are now device scope.
  - Other fixes for these cards: sppark's compute floor can be lowered by the app's opt-in (never below 6), and cards
    without memory pools fall back to `cudaMalloc`.
  - A build check now refuses any GPU module with system-scope atomics.
  - Measured on a GTX 1050 Ti: block 1 in 197.7 s, verified.
- **The installer ships the Visual C++ runtime beside the GPU prover**, so a PC without the redistributable can run it.
- **One worker per card**, pinned by the card's UUID. Home shows one line per card, including a stopped card and why.
- **The board decides which prover is accepted.** Start asks the board's `/api/meta`, off the window's thread, with a
  deadline. When the board can't be asked, the app says so and checks against the image it was built for.
- **Scheduled proving** (#725, #737): prove only inside a time window, finish the block in hand, keep the PC awake.
- **Stopping hands the block back.** A stopped worker releases its claimed block at once (`POST /api/release`, signed,
  naming its claim, #753) instead of holding it until the claim lapses. Close waits for the hand-back and names any block
  it couldn't return.

## Coordinator

- **Release by claim** (#753): a release names the claim it means. A late release can't free a block the same key has
  re-claimed. **Deploy the coordinator before the worker and the app.**
- **The genesis spine bootstraps on a fresh board** (#752). The replay guard had refused the seed receipt, so a new
  board's spine stayed empty. A failed seed is remembered and racing copies verify once.
- **The per-IP cap on new keys applies** (#738): `_client_ip` is set server-side on every POST.
- **The sponsor bot** (#746) skips a brief read-only hiccup before renting (exit 75) instead of latching off; a real
  outage still latches and alerts.
- The WASM verifier refuses a frame length that would wrap (#735).

## Build

- **The browser verifier builds reproducibly on any machine** (#751): inside a pinned container, at a fixed path, from a
  published image pinned by digest. CI checks the built wasm against its pin.
- Main's core verifier wasm is `996f1ed6` (it changed with #735's frame fix).

