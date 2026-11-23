# Worst-case valid blocks for the libzync prover (Hazync v2), 23.11.2026

The question: can a miner make one block that is valid under Bitcoin Core v28.0's rules but that Hazync cannot prove
(or cannot prove at a sane cost), which would stop the chain proof?

Every block below was built on regtest with Core's own `test_framework`, **accepted by Bitcoin Core 28.0
(`submitblock`)**, and then executed (dev mode, `RISC0_DEV_MODE=1`, nothing proved) by the guest through the command a
prover runs, `libzync-prove-range h h`, with the real coin log and trie from genesis (no modelled state). Labels:
**MEASURED** = printed by a run whose log is in `raw/`; **ESTIMATED** = arithmetic on measured numbers, stated with
its inputs.

## Answer in one paragraph

**Yes, at the shipped settings, a valid block can stop Hazync.** (1) **Memory:** a block whose inputs run
`OP_CHECKSIG` with empty signature and key 201 times over a 9.8 kB script made the guest abort with
`Out of memory! You have been using the default bump allocator which does not reclaim memory` at the default 2 chunks
and also at 8. Core accepted the block in 0.73 s. It proves only when split into more chunks: it ran at 32
and 128. The guest frees nothing (risc0 bump allocator, 3 GB address space). Core's legacy CHECKSIG copies the
scriptCode and FindAndDelete rebuilds it on every call, so heap grows with the number of signature operations
executed. The host's packer balances predicted cycles, not memory, so it does not avoid this.
(2) **Cost:** a full 1 MB block of bare SIGHASH_NONE CHECKMULTISIG inputs (24,377 inputs, 4,656,007 failing ECDSA
checks, each over a ~1 MB legacy sighash) is valid. Core 28.0 connected it in 506 s on 16 threads and in 7,622 s on
one. Its guest work is ESTIMATED from measured samples at **52.4 T user cycles, 56.7 T padded, about 652 GPU-days
on RTX PRO 6000s, about $39,000**. One input of it can cost up to **4.42 G user cycles (4.80 G padded, ~80 min on
one GPU), MEASURED**, and that work cannot be split.

## Hosts and images (MEASURED)

| | |
|---|---|
| box | `hazync-coord`, 16 cores, 62 GB, shared (load average recorded per run in each `TIMED` line) |
| Core | Bitcoin Core 28.0 release tarball, `sha256sum -c` against its `SHA256SUMS` OK; `test_framework` from the v28.x tree (1101837) |
| regtest host | built from **origin/main `75d053c9`** inside the published-image build container (`hazync-libzync-repro`, `reproduce/libzync.Dockerfile` paths and toolchain), `build_hosts.sh`. Mainnet build of the same tree prints **`46666b6f…`**, guest ELF `f7b87bf1…` = the published v0.27.1 image byte for byte; the regtest build (`HAZYNC_NETWORK=regtest`, the only difference) is **`eb2cd424…`**, ELF `1c3d671a…` (`raw/build/`). The guest source (patches, coreshim, methods) in that container image and in origin/main are file-for-file identical (71 files, sha256). |
| release host | v0.27.1 `hazync-host-x86_64-linux-gnu`, good PGP signature from `777FE81F…90F5B928`, `sha256sum -c` OK, sha256 `4c79a0f1…` (identical to v0.27.0's), `method-id` 46666b6f. Used for the cross-check below. |
| settings | `HAZYNC_CHUNKS` as stated (2 = shipped default), `HAZYNC_SEG_PO2=21` (the CUDA host's default, as the paper's cost curve), cost-packed chunks, signature hints by the host's own collect pass. |

Release-host cross-check (MEASURED, `xcheck_release.sh`, `raw/xcheck/agg_release.log`): the `none_k8` block with
its height relabelled to 900,000 through the published v0.27.1 host (`agg-exec-fake`, collect mode). Chunk 0:
**17,422,214,969** cycles (release, 46666b6f) against **17,378,442,807** (regtest image, hinted); chunk 1:
89,693,110 against 89,656,048. The +0.25% is the collect mode's per-signature log lines. The mainnet aggregate then
refuses the regtest header (`bv=false`), as expected. The scripts' cost is the same under the published image.

## The block constructions

All bare legacy scripts. A bare `scriptPubKey`'s sigops count in the block that **creates** it, never in the block that
spends it. The spending blocks below have 0 or ≤200 legacy sigops, so the 80,000 sigop limit never binds. This is
the hole BIP54's 2,500-per-transaction rule closes. BIP54 does not publish its own worst block: the author
withheld the construction (Delving #710; ~10 min on an i5-12500H, 2026). These are my constructions from the
rules.

- **HEAVY coin** (`heavy_script`): 6,644 bytes, exactly **201 non-push opcodes**:
  `9 × [OP_0 <sig> OP_1 <20 keys> 20 CHECKMULTISIG] + [OP_0 <sig> OP_1 <11 keys> 11 CHECKMULTISIG] + OP_1`. `<sig>` =
  strict DER r=s=1 + hashtype byte (passes DERSIG, never verifies). The keys are distinct valid points, so libsecp
  runs the full verify. m=1 against n keys tries every key: **191 legacy sighashes + 191 failing ECDSA verifies per
  input**, and the script still ends true. NULLFAIL, LOW_S and CLEANSTACK are policy, not consensus. The spend has an
  empty scriptSig, 41 bytes, so **24,377 such inputs fit one 1 MB block**. 191 checks is the maximum per 201-op script
  (CHECKMULTISIG costs 1 + n ops for n checks).
- **unsplit**: the same, but the scriptSig is also a heavy script (non-push scriptSigs are consensus-valid for bare
  outputs): **382 checks in ONE input**, the most one input can do (201 ops per script, two scripts).
- **MEM coin** (`mem_script`): 9,818 bytes: 18 pushes of 520 bytes, `OP_0`, `201 × (OP_0 OP_CHECKSIG)`, `OP_1`. Empty
  signature and empty key: CHECKSIG fails before any hashing or EC work, but Core first copies the scriptCode (9.8 kB)
  and `FindAndDelete` builds a new one (the `OP_0`s match) on **every** call.
- **LIGHT coin**: bare `OP_TRUE`, empty scriptSig: no signature work. Each sample block is ONE ~1 MB transaction with
  k heavy (or mem) inputs first, then light inputs to fill the block. In a legacy sighash every OTHER input is
  serialised blank (41 bytes), so a heavy input's sighash bytes are the same whether the other inputs are heavy or
  light. The per-input cost measured in a sample is therefore the per-input cost of the full block.
- **max_outputs**: one transaction with 111,051 empty-script outputs, the most coins a block can create.
  **light_only**: 24,374 light inputs, the most inputs. These two stress the trie and aggregate.

Why SIGHASH_NONE: the lever `HAZYNC_LEGACY_SIGHASH` (patches/libzync/0101) computes **only** hashtypes with neither
ANYONECANPAY nor NONE/SINGLE from preserialised slices with a prefix midstate. Every other type goes to Core's
serialiser. Even for ALL, the midstate covers only the inputs **before** nIn. The suffix `41 × (n − nIn − 1)`,
the outputs and the scriptCode are hashed on every signature. So the lever halves the quadratic term at most; it
does not remove it. OP_CODESEPARATOR and FindAndDelete only change the scriptCode, which is outside the midstate
anyway, so they do not defeat it. A non-ALL hashtype does. Measured per check, in a ~1 MB transaction (each about
1 MB hashed):

| hashtype | path | user cycles per check (MEASURED-derived) |
|---|---|---:|
| NONE (0x02) | Core's serialiser | **11,246,652** = (none_k24 − none_k8 chunk cycles) / 16 / 191 |
| ALL (0x01), input 0 | lever, preserialised | **3,227,067** = (unsplit − light_only) / 382 |
| ALL (0x01), inputs 0..7 | lever | 4,474,855 = (all_k8 − light_only) / 8 / 191 |
| NONE, in a 575-byte tx (~7 kB sighash) | | ≤ 140,000 (none_small_k8: 214.6 M for 2 chunks incl. their fixed cost, / 1,528) |

So the failing ECDSA verify plus FindAndDelete is ≤ 0.14 M of each check. **Sighash hashing is ≥ 98% of the cost.**
SIGHASH_NONE costs ~3.5× ALL per check because it bypasses the lever.

## MEASURED: every case (Core 28.0 verdict, guest verdict, per session)

Full per-session tables (every chunk, trie, aggregate; user, padded, segments) are in [`raw/table.md`](raw/table.md),
produced by `python3 parse.py raw/run/guest*.out`. Logs: `raw/run/out/<case>_c<chunks>.log`; driver output
`raw/run/guest1..7.out`; Core bench lines `raw/core/bench_lines.txt`.

| case | Core 28.0 | chunks | guest | user cycles (all sessions) | padded | largest chunk, padded (segments) | host peak RSS |
|---|---|---|---|---:|---:|---:|---:|
| light_only (24,374 inputs) | accept, 0.15 s | 2 | accept | 463,231,747 | 576,716,800 | 232,783,872 (111) | 1.28 GB |
| max_outputs (111,051 outputs) | accept, 0.38 s | 2 (1 used) | accept | 1,081,674,687 | 1,382,023,168 | chunk 350,224,384 (167); aggregate 671,088,640 (320); trie 360,710,144 (172) | 1.54 GB |
| none_k8 | accept, 0.60 s | 2 | accept | 17,655,864,452 | 19,181,600,768 | 18,826,133,504 (8,977) | 1.27 GB |
| none_k24 | accept, 0.83 s | 2 | accept | 52,025,665,407 | 56,378,785,792 | 56,019,124,224 (26,712) | 1.27 GB |
| all_k8 | accept, 0.52 s | 2 | accept | 7,309,812,565 | 8,117,026,816 | 7,761,559,552 (3,701) | 1.27 GB |
| none_small_k8 (575-byte tx) | accept, 0.02 s | 2 | accept | 217,372,810 | 231,997,440 | 114,294,784 (55) | 1.18 GB |
| unsplit (ALL, 382 checks, 1 input) | accept, 0.78 s | 2 | accept | 1,703,880,341 | 1,981,972,480 | 1,629,487,104 (777) | 1.27 GB |
| unsplit | | 8 | accept | 1,854,774,896 | 2,237,661,184 | **1,574,961,152 (751)**, the input alone | 1.66 GB |
| **unsplit_none** (NONE, 382 checks, 1 input) | accept, 0.83 s | 8 | accept | 4,890,406,095 | 5,458,886,656 | **4,796,186,624 (2,287), user 4,416,938,017**, the input alone | 1.66 GB |
| **mem_k1140** (1,140 mem inputs) | **accept**, 0.73 s | **2** | **ABORT** `Guest panicked: Out of memory! … default bump allocator which does not reclaim memory` (chunk 0 = inputs 0..10307) | – | – | – | 4.22 GB |
| mem_k1140 | | **8** | **ABORT**, same message (chunk 0 = 818 mem inputs) | – | – | – | 8.77 GB |
| mem_k1140 | | 32 | accept (204 mem inputs per chunk) | 23,190,171,321 | 31,743,049,728 | 5,117,575,168 (2,441) | 15.07 GB |
| mem_k1140 | | 128 | accept (51 per chunk) | 29,629,701,965 | 43,276,861,440 | 1,438,646,272 (686) | 11.39 GB |
| **full_none** (24,377 heavy NONE inputs) | **accept**: 506.1 s at the default threads (16 cores, load 17); **7,622.4 s at `-par=1`** (node restarted, cold caches, `reconsiderblock`) | not run in the guest (below) | | | | | |
| **full_all** (24,377 heavy ALL inputs) | **accept**: 583.5 s, default threads | not run | | | | | |

The guest's verdict matched Core's on every block it completed. Every case Core accepted, the guest accepted, or
aborted for memory. mem_k1140's 32/128-chunk runs re-submitted the same bytes, which Core answers
`duplicate-invalid` (I had invalidated it after its first acceptance, `raw/run/guest3.out`). The host RSS at 32/128
chunks is the host executing up to 16 chunks in parallel for the hint pass. It is not guest memory.

Memory, what is and is not measured: the guest has 3 GB (`GUEST_MAX_MEM = 0xC000_0000`, risc0-zkvm-platform 2.2.3;
the guest's `malloc`/`free` wrap the Rust allocator, which is the bump allocator: `free` is a no-op, asserted at guest
start). 818 mem inputs in one chunk ran out of memory and 204 did not. So one mem input leaves **between 3.7 MB
and 15 MB** of unreclaimed heap (MEASURED bounds; not measured more precisely). It costs ~18.9 M user cycles. A
full block of 24,377 such inputs (prep would need ~257 blocks; not built) would therefore need **at least ~120
chunks** to fit (ESTIMATED from the 204-input bound). The shipped default is 2.

## ESTIMATED: the full-size SIGHASH_NONE block in the guest

Not executed: at the measured executor rate (none_k24: 51.7 G cycles executed twice, collect + dev-mode prove, in 2,625 s ≈ 39 M cycles/s) one execution is ~15 days of CPU. Inputs: the per-heavy-input cost
**2,148,110,570** user cycles (none_k24 − none_k8, chunk sessions, MEASURED; none_k8 − light_only gives
2,147,983,535, so it is linear in k to 0.006%), the padded/user ratio of none_k24's heavy chunk (1.0826), and 24,377
inputs.

- chunk work: **52.36 T user cycles, 56.69 T padded** (+ trie and aggregate, ~0.23 G padded as light_only).
- GPU, the paper's fit (23.77 s + 0.9940 s per M padded cycles, one RTX PRO 6000, $2.49/h): **~56.4 M card-seconds
  ≈ 652 GPU-days ≈ $38,975** at any chunk count from 2 to 1,024 (the fixed 23.77 s is negligible). At 2 chunks each chunk is
  ~28.3 T padded = **~326 days on one card**. At 1,024 chunks, ~15 h each on 1,024 cards.
- Floor of one input (cannot be split): **unsplit_none, 4,796,186,624 padded → 4,791 s ≈ 80 min, $3.31 on one card**
  (MEASURED cycles, ESTIMATED time). A block's wall-clock can never be below that, whatever the number of cards.

What this estimate leaves out (unmeasured): (a) each chunk receives the transaction's whole prevouts blob. In
full_none that is ~162 MB of 6.6 kB scripts, against ~0.25 MB in the samples. Its per-chunk ingest cost and memory
are not measured, so the figures above are a floor for many-chunk splits. (b) Whether a 2-chunk split of full_none
(~12,000 heavy inputs per chunk) fits in 3 GB is **unknown**. Each heavy input also copies its 6.6 kB scriptCode 10
times and FindAndDelete rebuilds it 10 times, so it may run out of memory as the mem block did. (c) The host packer
calls `predicted_sig_ops` per input, and that deserialises the whole prevouts vector each time (O(n²) host work;
~160 MB × 24k for full_none). Its host time is not measured.

## What the paper can honestly say

- A Core-valid block exists that the shipped prover (2 chunks) **cannot prove: the guest runs out of memory**
  (MEASURED). It can be proved by splitting into more chunks (32 worked; ~120 is the estimated floor for a full-size
  version). The packer does not do this by itself, so today it is a liveness failure needing operator action.
  Fixing it needs either a memory-aware packer or a reclaiming allocator (an image change).
- The worst cost case is quadratic legacy sighash under a non-ALL hashtype. Core itself takes 506 s on 16 threads
  and 7,622 s on one thread for it (MEASURED). Hazync's ESTIMATED cost is ~56.7 T padded cycles, ~652 GPU-days,
  ~$39k per block, with a hard floor of ~80 min on one GPU for its most expensive single input. The midstate lever
  cuts ALL-type hashing ~3.5× per check but does not apply to NONE/SINGLE/ANYONECANPAY, which an attacker would use.
- This matches BIP54's concern. Under BIP54 (≤2,500 legacy sigops per transaction, counted on the spent
  scriptPubKey too) these constructions are invalid; Hazync's statement is Core v28.0's rules, which allow them.
- Trie, aggregate and many-output/many-input blocks are small: max_outputs (111,051 coins) is 1.38 G padded in total,
  light_only (24,374 inputs) 0.58 G (MEASURED).

## Reproduce (on hazync-coord, /root/worst-case)

```bash
bash build_hosts.sh src.tgz build regtest 8; bash build_hosts.sh src.tgz build mainnet 8   # src.tgz = git archive origin/main
HZ_ONLY=light_only,max_outputs HZ_CHUNKS=2 python3 -u worst.py build/host_regtest run guest > run/guest1.out   # also builds base + anchor
HZ_ONLY=none_k8 HZ_CHUNKS=2 python3 -u worst.py build/host_regtest run guest > run/guest2.out
bash run/chain3.sh; bash run/chain5.sh                                 # (raw/run/: unsplit, mem 2/8/32/128, all_k8, none_k24, none_small_k8, unsplit_none)
HZ_PORT=18790 python3 -u worst.py build/host_regtest full full > full/full.out                    # Core only: full_none (default + -par=1)
HZ_PORT=18790 HZ_FULL_ONLY=full_all HZ_PAR1=0 python3 -u worst.py build/host_regtest full full > full/full2.out
bash xcheck_release.sh run/blocks/none_k8/block_126.json rel/hazync-host-x86_64-linux-gnu /root/cost-curve/src/cost/txcounts.txt xcheck
python3 parse.py raw/run/guest*.out > raw/table.md
```

Notes on the run: the first `full` stage's client timed out (900 s) on the `-par=1` `reconsiderblock`. The node
finished it anyway and logged `Verify 24377 txins: 7622381.55ms` (`raw/core/bench_lines.txt`), and `worst.py` now
calls bitcoin-cli with `-rpcclienttimeout=0`. `chain3.sh`/`chain5.sh` ran `worst.py` / `worst_next.py`, earlier
copies of this `worst.py` without the later cases. Core accepted each case block, then I invalidated it so the next
case builds on the same tip.
