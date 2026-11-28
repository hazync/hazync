# zeroconf regtest fixture (26.11.2026)

On the zAPP regtest harness (scripts/zapp-regtest), main branch, tip 110 `2d5cc9c8...`:
- `T.json`: a spend of ALICE's P2WPKH output `bffc2f43...:3` (block 102) and P2PKH output `aa970d97...:0` (block 105),
  both unspent at 110 by Core's own gettxout (oracle/snap_110.json), to her P2WPKH, fee 10,000 sat; signed with Core's
  test_framework, RFC 6979 (`../make_fixture.py`: two runs give the same bytes). txid `6c00b169...`.
- **Core v31.1 accepts it** on top of 110: `testmempoolaccept` allowed (`../core_accept.py`: a throwaway regtest node
  fed the harness's blocks 1..110).
- `template_111.json`: the block fixture for 111 holding T (fetch_template_rpc.py's schema).
- `chunk_111.journal`: its SPECULATIVE chunk (HAZYNC_SPECULATIVE=1, HAZYNC_CHUNKS=1, the regtest host): all_valid 1,
  height 111, zero hash and time, MTP = the tip's (1296751602), one record = T (fee 10,000, weight 1,031 = Core's,
  sigops 1). The ENGINE's chunk journal for the same stream (keys-rtch) is byte-identical.
- `paths.json`: each input's coin-log path at 110 (`zync-path find` / `coin` on the harness chain's history.bin, from
  native prep with HAZYNC_PREP_HISTORY=1, which gives the proved engine run's history.bin byte for byte on mainnet
  1..200): log positions 105 and 175, both unspent.

Found by it: the zeroconf guest parsed the chunk record from before the trie's script hashes and the undo digest
(#729) were added: fixed in prover/zyncapps/methods/zeroconf/src/main.rs.

## Run on the engine (26.11.2026 late)

`zyncapp prove-zeroconf <prep state_110.json> chunk_111.journal 0 paths.json out --regtest --stream zc.bin` (12,700
bytes) -> `hzengine execute` on the zeroconf guest with the parser fix, METERING build (keys-z-zeroconfb:
bench-noverify, env::verify accepts the two journals without proofs, so this run tests the zAPP's own checks; the
production build keys-z-zeroconf2 compiles too): **EXECUTE_OK**, statement 840 bytes: kind 0x5A430002, tip 110
`2d5cc9c8...`, profile 0xf9, **txid 6c00b169... and wtxid f4a83537... = T's, fee 10,000, weight 1,031, sigops 1**.

Negatives (`../negatives.sh`), each refused by the guest with its own reason: paths swapped between the inputs, a path
node changed (both "an input's coin is not unspent at the tip"), one path for two inputs, the tip one block lower,
a chunk for another height (both "not for the block after this tip"), a chunk not all valid, no such record, the
wrong network ("wrong network"); the control (the fixture) accepted.

**With REAL child proofs, on CPU** (`../real_proofs.sh`, build box): block 110 proved by the regtest core image
(keys-rtch) as a base step on native prep's state at 109 (368 s); the speculative chunk proved by the same image
(140 s; its journal = the fixture's); zeroconf proved by the PRODUCTION image (keys-z-zeroconf2) with both as children,
verified in-guest (161 s); `hzengine verify` OK. Statement: kind 0x5A430002, tip 110 `2d5cc9c8...`, core_id = keys-rtch's
image id, txid / wtxid = T's, fee 10,000, weight 1,031, sigops 1. (A tip proved from an anchor, not from genesis:
enough for the zAPP, whose statement carries the anchor_id for its verifier to check; the chain from genesis is
the core prover's, tested on mainnet 1..200.)

## Two more cases, each with its own transaction and chunk (27.11.2026)

`../make_fixture.py <zr> <tf> <out> spent|parent` and `../cases.sh` (build box, metering image; output: cases.out).
The default fixture is unchanged by the new code: T.json and template_111.json were rebuilt byte-identical.

| case | Core v31.1 (testmempoolaccept at 110) | the zeroconf guest |
|---|---|---|
| **spent**: T spends ALICE's P2PKH coin of block 102, which block 105 spent (log position 104, spent_by 105) | refused, "missing-inputs" | refused: "an input's coin is not unspent at the tip" |
| **parent**, record 0: T, in a block that also holds T2 | allowed | ACCEPTED (the control) |
| **parent**, record 1: T2 spends T:0 in the same block (its coin claims height 111, flag bit 1) | allowed as a package with T | refused: "an input spends an unconfirmed coin" |

The speculative chunk is all_valid in all three cases: it checks the scripts, and the zAPP checks the coins. The last
row is a deliberate difference from Core: an unconfirmed parent's coin is not in the coin log, so the zAPP cannot
prove it unspent, and refuses it (the guest's own doc comment). A merchant who needs it waits for T to confirm.

The verifier side (27.11.2026): zapps/zeroconf, the module `zapp_zeroconf`. It checks the real proof above under the
production image's pinned key, with keys-rtch as the trusted core and the state at 109 as the anchor: DEMO (a given
anchor), T's txid, fee, weight and sigops; 13 refusals (tests/engine.rs).

**Own kind, 27.11.2026:** the guest commits 0x5A5A0001 (it shared the coin zAPP's 0x5A430002). Rebuilt as keys-z-zeroconf3
(production, 1ee5f2d7...) and keys-z-zeroconfc (metering); everything above re-run on them with the same results:
the 8 negatives refused and the control accepted, both cases, and the real proof (real_proofs.sh -> real3/: core 354 s,
chunk 126 s, zeroconf 155 s, verified; statement kind 0x5a5a0001).

## The genesis anchor pinned in the guest (28.11.2026, review R2-6 #2, #4)

Commit 977439f0: the guest checks the core statement against the network's PINNED genesis anchor (mainnet 852c5c7e...,
signet 3b67ead0..., regtest 99509fc5...: the regtest one from scripts/libzync/genesis_anchor_id.py, zapps/corecheck's
REGTEST_IMAGE and the anchor_id native prep's regtest states carry), where it passed the journal's own anchor_id (a
no-op). Its doc comment says consensus-valid for the block after the tip, no relay policy.

- **Production image** (reproduce/engine-ids.sh IMAGES=zeroconf --rev 977439f0, the container, build box):
  **132969d821982852161a4a4a92833980438992d12e0e21cf9695cf98c9629084** (exe 68ad2f6e..., vm 00e00077...; keys as
  keys-z-zeroconf3's: agg.vk 15d415ec..., sdk.pk f0155abe...). Replaces 1ee5f2d7.
- **Metering build** of the same source (exe 70560b01...; built natively with the box's older hzengine, then marked
  BENCH_NOVERIFY as today's builder marks it), today's hzengine: `negatives.sh` with two new cases
  (negatives_28.11.2026.out): the control (prep's state at 110, anchored at regtest genesis) EXECUTE_OK, journal 840
  bytes (control_28.11.2026.out); the 8 earlier negatives refused with their reasons; NEW: the tip's anchor_id with one
  bit changed, and the tip carrying mainnet's genesis anchor: both refused, "core statement: not anchored at the
  network's genesis". (The control first read "refused" with no reason: the host predated review R1-1 #1, then the
  keys dir lacked the BENCH_NOVERIFY mark; neither is a guest refusal.)
- **With REAL children** (real_anchor.sh, real_anchor_28.11.2026.out): block 110 proved by keys-rtch as a base step on
  prep's state at 109 (1,025 s, 4 cores), so ANCHORED AT 109, and the fixture's speculative chunk for 111; the new
  production image, both children verified in the guest first: refused, "core statement: not anchored at the
  network's genesis" (src/main.rs:122).

So the real-proof fixture (zapps/zeroconf/tests/data/engine: image 1ee5f2d7, its core child a base step at 109) can
no longer be made with the production image: a zeroconf proof now needs a core chain proved from genesis. The
reader's tests keep the 1ee5f2d7 proof (it verifies as that image, which is what they test). A real proof of the new
image waits for the regtest harness's chain proved from genesis on the engine (110 blocks; the queued box item).
