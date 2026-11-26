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

Left: a spent input and an unconfirmed parent as fixture cases (both need their own chunk); the zeroconf verifier
side (`zyncapp verify-zeroconf` on an engine proof: the statement's checks against a verified core state).
