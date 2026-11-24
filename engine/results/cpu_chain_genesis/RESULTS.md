# Genesis chain on the engine through the PRODUCT host (CPU, build box, 24.11.2026)

`engine/scripts/chain_cpu.sh`: product host (engine branch) with `HAZYNC_ENGINE`, guest keys-hz2.
- blocks 1..2 from genesis: block 1 a base step on the genesis anchor (554 s); block 2 verifies block 1's proof and
  its trie proof in-guest (572 s)
- block 3 as a base step on S(2) (465 s); `libzync-join` of block 2 + block 3: both verified in-guest, adjacency
  checked (286 s) -> one proof of 1..3 anchored at genesis
- `libzync-verify` on block 1, block 2 and the join: every check ok, anchor genesis; block 3 ok against its anchor.
  Verdict DEMO as for any product proof below the pinned checkpoint / minimum chain work (expected at height 3).
CPU times on a shared, oversubscribed box (load ~50 on 16 cores); not a performance number.
