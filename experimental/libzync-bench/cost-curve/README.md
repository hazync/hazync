# Cost curve: Hazync v2 (libzync) proving work across mainnet history, from the published v0.27.0 host

Execute-only (dev mode / `agg-exec-fake`) cycle counts of real mainnet blocks from height 170 to 976,000 with the
**published** v0.27.0 Linux host (image `46666b6f`), on the CPU of `hazync-coord` (16 cores, 62 GB, shared: its load
average was 25-33 from other work throughout, recorded per run). No GPU, nothing proved. Results, the curve, the
card-time projection and its uncertainty: [`RESULT.md`](RESULT.md). Raw outputs: [`raw/`](raw/).

Two kinds of measurement, labelled everywhere:

- **(A) REAL STATE, exact.** The leaf bundles `leaf_<h>.bin` that the paper prep on `hazync-proof` cut from its
  stores (`libzync-prep` from genesis: the exact S(h-1), and the BIP30, coin-log and trie witnesses against it, plus
  block h). Each is executed end to end by the command a board prover runs (`prove-range-bridge h` =
  `libzync-prove-range h h <dir> leaf <out>` with `HAZYNC_LEAF_BUNDLE`, `prover/host/src/lz_coord.rs`), in dev mode
  (`RISC0_DEV_MODE=1`: every guest run executes and every claim is checked, the receipts are fake). Per part the host
  prints `LZ_STATS <part> segments= total_cycles= user_cycles=`; `total_cycles` is the padded count (sum of 2^po2 over
  segments, what a prover pays for). The committed state (decoded from the aggregate's journal) is compared field by
  field with prep's own `state_<h>.json`: every field matches except `anchor_id`, which differs by construction (a
  leaf is a base step on S(h-1), so its anchor is S(h-1)'s digest; prep's chain carries genesis's).
- **(B) BLOCK WORK REAL, STATE MODELLED.** The block file (transactions + prevouts) from the archive node with
  `prover/fetch_block_rpc.py`, then exactly `../profiles/run.sh`'s v2 commands (`libzync-hints`, `agg-exec-fake`):
  the chunks are the real block; the trie update runs against a synthesised old trie (`trie_model::synth`) and the
  aggregate against a synthesised coin log (`lz::cl_synth`, size k = 1.64 x cumulative transactions,
  `../cost/txcounts.txt`). **Trie and aggregate are MODELLED** in every (B) row.

Settings shared by A and B: the shipped chunking (`HAZYNC_CHUNKS` unset: the host default, 2 chunks, cost-packed,
as the board worker runs it, `coordinator/hazync` sets no chunk count) and `HAZYNC_SEG_PO2=21` (the CUDA host's
default, so segment counts are a GPU prover's; the CPU host's own default is 20). User cycles do not depend on po2.

## Binaries and inputs

| | sha256 |
|---|---|
| v0.27.0 `hazync-host-x86_64-linux-gnu` (224,140,328 B), Good PGP signature on `SHA256SUMS.txt` from `777FE81F…90F5B928`, `sha256sum -c` OK, `method-id` = `46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c` | `4c79a0f144e06b4d1d66b3af775c372687ffa562c2c06ed4bff97318de45e82b` |
| `../cost/txcounts.txt` (B's coin-log size model input, unchanged) | `e29f03f010b6edb5a74f6e55e3e7cd1e8ff4c51518c11b3f389b1d6124ae756c` |
| `raw/txcounts_5k.txt` (getchaintxstats every 5,000 blocks to 976,724; projection input) | `1df6b0f612c4f3114944ef361a5d340b5f841e7318de6c7ca514fad276b91a1e` |
| leaf bundles and block files | `raw/inputs.sha256` |

The leaf bundles were copied read-only from `hazync-proof:/srv/bulk/libzync-prep/leaves/` (prep unit
`libzync-prep.service`, `HAZYNC_PREP_LEAF_AT=170,1000,100000,…`) with `nice -n 19 ionice -c3 tar`, with prep's
`state_<h-1>.json` and `state_<h>.json` from `/var/lib/libzync-fast/out/`. The prep had reached ~458k when they were
copied, so (A) stops at 400,000. Block files: `fetch_block_rpc.py <h> block_<h>.json` against `hazync-coord`'s own
bitcoind (txindex, unpruned, tip 976,724). The fetched 962,000 has transactions and prevouts identical to the repo
fixture `prover/block_962000.json`; it differs only in three header fields the fetcher now adds and the order of
`recent_times`.

## Commands (on hazync-coord, `/root/cost-curve`)

```bash
# host
gh release download v0.27.0 -R hazync/hazync -p hazync-host-x86_64-linux-gnu -p SHA256SUMS.txt -p SHA256SUMS.txt.asc -D bin
(cd bin && gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt && sha256sum -c --ignore-missing SHA256SUMS.txt)
# inputs (from the laptop: read-only on hazync-proof, niced)
ssh hazync-proof 'cd /srv/bulk/libzync-prep && nice -n 19 ionice -c3 tar ch leaves/leaf_<h>.bin out/state_<h-1>.json out/state_<h>.json …' \
  | ssh hazync-coord 'cd /root/cost-curve/inputs && tar x'
for h in …; do python3 -I fetch_block_rpc.py $h blocks/block_$h.json; done
blockstats.sh <heights> > blockstats.tsv          # getblockstats: txs, ins, outs, weight
txcounts.sh 5000 976724 > txcounts_5k.txt        # getchaintxstats
chainsample.sh 1000 976724 > chainsample_1k.tsv  # getblockstats of one block per 1,000 (the projection's inputs/outputs)
# runs, one at a time
run_a.sh bin/hazync-host-x86_64-linux-gnu inputs runA        # (A)
chain.sh                                                      # (B) curve + dispersion, the 962,000 setup check, profiles
# tables and projection
python3 parse.py runA runB blockstats.tsv raw/
python3 project.py raw/table_a.tsv raw/table_b.tsv raw/txcounts_5k.txt 976724 raw/chainsample_1k.tsv > raw/projection.out
python3 -I prof_split.py <profile.pb …>
```

`timed.py` wraps each host command (wall, CPU, peak RSS, load average at start and end). `cmp_state.py` is the (A)
state check. `prof_split.py` and `capture_pprof.py` are `../profiles/`'s (branch `docs/evidence-v027`, `c74d042`),
copied unchanged; `prof_split.py` reads `../cost/core_share.py`.

`chain.sh`'s setup check runs block 962,000 exactly as `../profiles/run.sh` does (the fixture, one chunk, no
`HAZYNC_SEG_PO2`, so po2 20), to reproduce that file's published numbers; every other run here is at po2 21.

## Raw outputs (`raw/`)

| file | what |
|---|---|
| `runA/leaf_<h>.log`, `runA/times_<h>.tsv` | (A) host output (`LZ_STATS`, `TIMED`, `STATE_MATCH`), the host's per-phase times |
| `runB/hints_<h>.log`, `runB/agg_<h>.log` | (B) host output of `libzync-hints` and `agg-exec-fake` |
| `table_a.tsv`, `table_b.tsv` | `parse.py`'s tables: per part user / padded cycles and segments, wall, RSS, load, block stats |
| `projection.out` | `project.py`'s output (all four interpolations, three rates, per-era table, dispersion, fit residuals) |
| `prof/<A|B><h>.<n>.pb`, `decoded.txt` | profiles and their decoding. A (leaf): .0/.1 signature-hint collect runs, .2/.3 the two chunks, .4 aggregate collect run, .5 trie update, .6 aggregate. B (agg-exec-fake): .0/.1 chunks, .2 coin-log collect run (host-side model, never proved), .3 trie, .4 aggregate |
| `repro/` | the 962,000 setup check |
| `blockstats.tsv`, `chainsample_1k.tsv`, `txcounts_5k.txt`, `inputs.sha256` | block content, chain samples, transaction counts, input hashes |
| `runA.out`, `runB.out`, `chain.out`, `prof.out` | the drivers' console output |
