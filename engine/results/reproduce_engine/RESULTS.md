# Reproducible engine image ids (ROADMAP 4.1 groundwork, 27.11.2026)

`reproduce/engine-ids.sh` builds the engine core image and the zeroconf zAPP image of a commit in a pinned container
(`reproduce/engine.Dockerfile`) and prints each image's exe commitment, vm commitment and image id SHA-256(exe || vm).
Nothing here is published: no record in swiftsync-verify's IMAGES and no LIBZYNC_IDS line. The ids are chosen at the
Stage 4 freeze.

## Result

Commit 92e23ef0, on two machines (the laptop, WSL2; hazync-coord, a fresh clone), both in the container:

| image | image id | exe | vm | guest.vmexe sha256 |
|---|---|---|---|---|
| core (mainnet, default features) | c7fe0eb8 5d2d4c10... | 0ddcb255... | 00e00077... | a2734c5a... |
| zeroconf | 1ee5f2d7 a8d10a2a... | b16bc53d... | 00e00077... | 0c7b1509... |

- The two machines agree byte for byte (the vmexe files too), and both equal the build box's native reference builds:
  keys-ch (c7fe0eb8, the image of swiftsync-verify/tests/data/engine200) and keys-z-zeroconf3 (1ee5f2d7, the image of
  zapps/zeroconf/tests/data/engine), vmexe sha256 equal to those dirs' guest.vmexe.
- Keygen in the container (`hzengine keygen`, HZ_GUEST_MEM_BITS=29, 33-35 s) gave the box's keys-rt29 byte for byte:
  sdk.pk f0155abe..., agg.vk 15d415ec..., openvm.toml 75b745db.... The vm commitment depends only on these keys.
- Time per run: laptop 7 min 16 s, box 5 min 49 s (image cached, cargo cache warm); the box from an empty cargo cache
  13 min 35 s (image build included); the laptop's first image build 26 min (a slow link: the image fetches two Rust
  toolchains and RISC Zero's 304 MB riscv32 C toolchain).

## What reaches the id

- **exe**: the guest program (ELF -> VmExe). Inputs: the guest source (assembled from prover/methods/guest, libzync,
  coinbase-smt and the engine's edits), the C/C++ base (Core v28.0 and libsecp v0.5.1 by commit, plus patches), the
  guest toolchain nightly-2026-02-28, the riscv32 gcc/g++ 13.2 (RISC Zero toolchain release 2024.01.05; its gcc and cc1
  are byte-identical on the laptop, the box and in the release tarball), the guest's crates (pinned:
  reproduce/engine-locks), and two PATHS (below).
- **vm**: poseidon2 over the app, leaf and internal-for-leaf vk commitments: the keys alone (the VM config, guest
  memory 2^29, the SDK's parameters). No guest input. Not the heavy part: keygen is ~35 s on CPU, sdk.pk 1.6 MB.
- The host toolchain that builds hzengine (1.97.1 here, the box's) did not move anything: HEAD's hzengine rebuilt the
  26.11 source tree to keys-ch's exe exactly. Not tested: another host toolchain.

## Two paths reach the exe (measured on the build box)

1. **The engine kit's path.** The guests' platform crates (engine/platform, verify-core) are path dependencies at the
   kit's absolute path, and cargo hashes a path dependency's location into its `-C metadata` (symbol names), which
   `--remap-path-prefix` does not touch. Same source, same everything else: kit at /root/engine-kit/engine gives exe
   0ddcb255... (keys-ch), at /root/r41/src/engine 56d9b208.... The box's references were built with the kit at
   /root/engine-kit, so the container stages the checkout there. (FEASIBILITY Q15's two builds had the same kit path.)
2. **A zAPP guest's location.** engine.rs `remap_flags` maps `guest/../../..` as "the tree". For the core guest
   (asm/prover/methods/guest) that is the assembled tree's root; for a zAPP guest (one directory) it is the directory
   two above it. zeroconf assembled under /root/engine/zg/ gives exe b16bc53d... (the leaf name does not matter: two
   names, same exe), under /root/r41/deep/a/b/ 2c7ab404..., at /hz/zc 4bc80f6d.... The container assembles it under
   /root/engine/zg/, as the reference was.

The assembled CORE tree's own path does not reach the id (remapped): the 26.11 tree rebuilt at /root/r41/old gave
keys-ch's exe.

## The guests' locks

engine/guest/assemble.sh and engine/zapps/assemble_guest.sh write no Cargo.lock, so a build resolves crates.io at build
time. The container builds with OPENVM_BUILD_LOCKED=1 (cargo --locked) from reproduce/engine-locks: zeroconf's is the
box's lock behind keys-z-zeroconf3; the core's is the box's lock behind keys-ch, extended for HEAD's manifest (7aca585c
added the optional verify-core dependency, which a lock lists) without any package changing version.

## Open, for the freeze

- Make both paths not matter, or decide them: a layout-independent remap for zAPP guests (moves every zAPP id once), and
  either a fixed kit path in the published recipe (as now) or a build that copies the platform crates into the
  assembled tree. Either way the published ids are the container's.
- The other images: utreexo, utreexo-tip, hints, headers, the handover verifier, signet/regtest variants. The script
  takes core and zeroconf only.
- The assemble scripts could write the pinned lock themselves, so a native build cannot drift either.
- A third machine, and a second host toolchain.
- reproduce/engine.Dockerfile pulls apt packages from snapshot.ubuntu.com: one box run failed on a transient 503 and
  passed when re-run; apt retries would help (a Dockerfile change gives a new image tag, nothing else).

## Every engine image, and the assemble scripts pin the lock (27.11.2026, later)

`reproduce/engine-ids.sh` builds any of eight images (IMAGES="core core-signet zeroconf hints utreexo utreexo-tip
headers handover-verifier"), each assembled at the build box's reference layout (core /hz/asm, zAPPs
/root/engine/zg/<zAPP>, headers /root/engine/hdr/guest29, the handover verifier /root/engine/hv/w) with its lock pinned
in reproduce/engine-locks (the box's locks behind the reference keys dirs, 26.11.2026). core-signet is the core guest
with the `signet` feature and HAZYNC_NETWORK=signet (the guest's build.rs requires both). Laptop only (the box was busy).

| image | run 1 (ab376686) | run 2 (750db597) | reference (build box) |
|---|---|---|---|
| core | c7fe0eb8... (exe 0ddcb255) | the same | keys-ch c7fe0eb8: equal |
| core-signet | d8ed9383... (exe e7490552) | the same | none built |
| zeroconf | 1ee5f2d7... (exe b16bc53d) | the same | keys-z-zeroconf3 1ee5f2d7: equal |
| hints | 84b54258... (exe d1c3c572) | the same | keys-z-hints 84b54258: equal |
| utreexo | 340f6675... (exe feb4b140) | the same | keys-z-utreexo 340f6675: equal |
| utreexo-tip | 6ecf6f43... (exe b640b916) | the same | keys-z-utreexo-tip 6ecf6f43: equal |
| headers | 25b32ac5... (exe a080482e) | the same | keys-hdr29 25b32ac5: equal |
| handover verifier | 7992e373... (exe a5382774) | the same | keys-hv 7992e373: equal |

vm 00e00077... for all (the 2^29 production keys; keygen 33.4 s, sdk.pk f0155abe, agg.vk 15d415ec). Every reference
agrees, so no input differs. Run 1 and run 2 give the same ENGINE_IDS line for every image (exe, vm, guest.vmexe
sha256) and the same baseline.json; each run 808 / 811 s on the laptop with a warm cargo cache. Full ids:
reproduce/out-engine after a run, or the table's run directories in the session.

The reference images were built on 26.11.2026 morning; later source changes (libzync/src/coinlog.rs 90d8fef0, the
zAPP manifest's `bench-noverify` feature ecfbf940) did not move hints', utreexo's or utreexo-tip's ids (measured above).

**The assemble scripts write the pinned lock** (750db597): engine/guest/assemble.sh (not for an ovm-xver build, whose
manifest differs), engine/zapps/assemble_guest.sh (when the zAPP has one), engine/zapps/headers/assemble.sh and
engine/handover/assemble.sh copy reproduce/engine-locks/<image>.Cargo.lock; HZ_UNPINNED_LOCK=1 leaves it out (a
manifest change that needs a new lock). Checked natively: each zAPP's and the headers guest's assembled lock is
byte-identical to the pinned one; with HZ_UNPINNED_LOCK=1 none is written. Run 2 builds on that commit: ids unchanged.

Still open: a second machine for the six new images IN the container (the references agree, but they are native box
builds, a second machine only for the native path); the layout-independent remap (moves every zAPP id: a freeze-time
decision); a third machine and a second host toolchain; regtest-test (not built: a test image).
