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
