# MMR paths for the Block check (hazync.org/headers/)

`hazync-headers-api` answers "the MMR path of block h" for the header-chain proof that `hazync-headers-follow`
publishes (`../headers/`), so the Block check on hazync.org/headers/ can show that block X at height h, or a
transaction in it, is on the proven chain from genesis.

**It is untrusted by design.** The browser verifies the published receipts itself, then checks every answer against
the MMR peaks those receipts commit to (`verifyHeaders({..., include})`, the same checks as `zapp-headers
--include ... --tx <txid>:<answer.json>`; the header-chain zAPP, `zapps/headers`):

- **`/path`**: the hash the READER asked about (not the answer's `hash`), at the height asked, with the answer's
  `path`, must climb to the committed peak for that height; the path's length is fixed by the height and the tip.
- **`/tx`**: the same for the transaction's block, and: the 80-byte `header` hashes to that block; the `raw`
  transaction parses, is not 64 bytes, and hashes to the txid the READER asked about (the answer's `txid` is not
  read); the raw `coinbase` is a coinbase whose `coinbase_branch` reaches the header's merkle root, which pins the
  tree's depth; and `branch`, exactly as deep, leads from the asked txid at `pos` to the same root
  (`verify_tx_in_header` explains the 64-byte attack the coinbase closes).

`leaves`, `mmr_root`, `tip_hash` and the answer's `hash` are for display and debugging; the verifier derives what it
needs. A lying service can refuse or answer wrongly, and the page says "refused"; it cannot make a block or a
transaction look included. It only saves the browser from holding a million block hashes. In the browser, read
`included` with the verdict: it is true only for an `ok` chain; `included_demo` says the same for a `demo` one.

## What it serves

| GET | Answer |
|---|---|
| `/path?h=<height>[&tip=<t>]` | `{height, hash, tip, tip_hash, leaves, path, mmr_root}` |
| `/tx?txid=<txid>[&tip=<t>]` | the above for the transaction's block, plus `{txid, pos, header, raw, branch, coinbase, coinbase_branch}` |
| `/status` | `{ready, tip, tip_hash, leaves, mmr_root, error}` |

`tip` defaults to the published tip; any earlier tip works too (a page that verified an older `runs.json` asks for
its own tip). Hashes are reversed hex as explorers print them, `path` entries raw-order hex (as `zyncapp
headers-mmr-path` prints them), branches reversed hex (as Electrum's `blockchain.transaction.get_merkle`). The `/tx`
answer is exactly what `zapp-headers --tx <txid>:<file>` and `include.tx` (with `include.txid`, the
transaction asked about) in the browser read.

The MMR is at the tip the follower **published** (`$HEADERS_DIR/published`), never the node's newest block, so a path
leads to the peaks of the receipts the page reads. It is rebuilt from the node's block hashes (batched JSON-RPC,
cookie auth) and cached in `$STATE_DIR/hashes.bin`; a restart reads the cache. New hashes are taken only with their
80-byte headers, LINKED (each hashes to its hash and names the one before, down to the last held hash), so a reorg
during a fetch is seen as a break and the range is fetched again, rather than leaving old-branch hashes in the middle.
A reorg below what is held drops the held hashes back to where the node agrees. While the held hashes do not end on
the published hash nothing is served (503).

**`/tx` needs `txindex=1` on the node** (`getrawtransaction <txid>` must find a confirmed transaction's block). Each
`/tx` costs the node `getblock <hash> 1` (the txids, not verbosity 2's whole block), the raw header and the raw
coinbase; the last `TX_CACHE` (16) blocks are cached. `/tx` has its own RPC connection, so a slow one never holds up
the sync, at most one talks to the node at a time (others wait up to 10 s, then 503), and it is limited harder than
`/path`: `TX_RATE` 0.2/s, `TX_BURST` 5 per client.

**A stale cookie.** Under systemd the cookie is the `LoadCredential=` copy made at start. If bitcoind restarted
without this unit (`PartOf=` normally restarts it too), every call is answered 401; after `AUTH_FAILS` (3) syncs in a
row the service exits 3 and `Restart=on-failure` starts it again with a fresh copy.

## Install (hazync-proof; done 20.11.2026, v0.27.0)

Network, as decided: the same pattern as the coordinator's `:8899`. The service listens on hazync-proof's own address
(`BIND=159.195.207.224`, port 8902, set in the unit), nginx on the web box (152.53.86.216) proxies
`https://hazync.org/headers-api/` to it, and **nftables lets only the web box reach the port**. `TRUSTED_PROXIES`
(in the unit) is the web box, so the per-client rate limit reads its `X-Forwarded-For`.

```sh
install -m755 hazync-headers-api /opt/hazync-headers/bin/
cp hazync-headers-api.service /etc/systemd/system/
# nftables: only the web box may connect to 8902 (put it in the table that holds the :8899 rule, before any accept-all)
nft add rule inet filter input ip saddr 152.53.86.216 tcp dport 8902 accept
nft add rule inet filter input tcp dport 8902 drop
# ...and persist it the way the :8899 rule is persisted on that box (e.g. /etc/nftables.conf)
systemctl daemon-reload && systemctl enable --now hazync-headers-api
curl -s 159.195.207.224:8902/status                 # from hazync-proof
curl -s -m 5 159.195.207.224:8902/status            # from anywhere else but the web box: must time out
```

`DynamicUser=yes`: it runs as a throwaway user that reads the node's cookie through `LoadCredential=` (the unit
restarts with bitcoind, which writes a new cookie) and `/var/lib/hazync-headers/published` read-only (that file must
stay world-readable, as the follower's default umask makes it). It needs nothing else. The first sync of the whole
chain fetches 975k hashes and their 80-byte headers (for the linkage) in batches of 2,000 and hashes ~2M MMR nodes in Python; later syncs fetch only new blocks.

## Behind nginx (applied on the web box, 20.11.2026)

The page's CSP is `connect-src 'self'`, so the service must be same-origin: **`https://hazync.org/headers-api/`**
(not api.hazync.org, which would need a CSP change). In `deploy/nginx-hazync.org.conf` of hazync-web, next to
`/headers-live/`:

```nginx
    # MMR paths for the Block check on /headers/ (hazync repo, coordinator/deploy/headers-api/). Untrusted: the page
    # checks every answer against the proof it verified. GET only; limited like /api/. hazync-proof lets only this box
    # reach :8902 (nftables), as for the coordinator's :8899.
    location /headers-api/ {
        limit_except GET HEAD { deny all; }
        proxy_pass http://159.195.207.224:8902/;
        proxy_http_version 1.1;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_connect_timeout 5s;
        proxy_read_timeout 30s;
        limit_req zone=hazyncorg_api burst=10 nodelay;
        limit_conn hazyncorg_conn 10;
        limit_req_status 429;
        limit_conn_status 429;
        add_header Content-Security-Policy "default-src 'none'; frame-ancestors 'none'" always;
        add_header X-Content-Type-Options nosniff always;
        add_header Cache-Control "no-cache" always;
    }
```

## Test

`test/test_api.sh` runs the service against `test/fake-bitcoind-rpc`, a fake node serving real mainnet headers
0..200 and real transactions (`experimental/libzync-bench/headers/tx/`). It checks:

- syncing to the published tip (not the node's newest), and following it;
- paths equal to the independent reference (`mmr_path_{37,170}_tip200.txt`), roots at 101 and 201 leaves;
- paths for an earlier tip;
- the `/tx` answer equal to the node fixture;
- refusals: 400 for bad input, 404 out of range or for an unknown transaction, 405 for writes;
- the cache on restart;
- a reorg against the published hash (503, then recovery);
- the cookie, and exit 3 after `AUTH_FAILS` refused syncs in a row (for systemd to reload it);
- per-client rate limiting behind a trusted proxy, and the tighter `/tx` limit;
- `/tx` asks `getblock` with verbosity 1 only, and a second transaction of the same block comes from the block cache;
- a slow `/tx` (a 5 s `getblock`) does not hold up the sync;
- a reorg during the fetch (an old-branch hash once, with a real-looking header): seen by the header linkage, the
  range fetched again, block 37's path (over the reorged 120) equal to the reference.

With `HEADERS_RECEIPTS=<0..100>:<101..200> HEADERS_IMAGE=<id> VERIFY=<zapp-headers>` it also verifies the
service's answers with the real verifier against receipts of the MMR image, with `--tx <the asked txid>:<answer>`,
and refuses the service's answer for block 170's coinbase given as the answer for the asked transaction.
