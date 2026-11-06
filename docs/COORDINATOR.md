# The coordinator

Merged from `COORDINATOR.md` (2026-09-14), `COORDINATOR.md` (2026-09-27) and
`COORDINATOR.md` (2026-09-17), now in `docs/history/`. Generated API and configuration reference:
[`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md). The code is
authoritative: `coordinator/server.py` (HTTP API, configuration), `coordinator/hazync` (worker CLI, shipped
as `hazync-worker`), `coordinator/run-workers.sh` (launcher, shipped as `hazync-run-workers.sh`),
`coordinator/sponsor_bot.py` (sponsor bot), `coordinator/deploy/` (units; see `coordinator/deploy/RUNBOOK.md`).

## 1. What the coordinator does

The **board coordinator** is a long-lived service that hands out proving work (blocks), verifies submitted
receipts, keeps the board and scoreboard, takes payment for sponsored blocks and serves proofs and witness
bundles. It never proves and needs no GPU.

> Not to be confused with the **segment coordinator** ([`OPERATIONS.md`](OPERATIONS.md)): an ephemeral
> `seg-serve` process that splits ONE block's proof across machines, lives for one prove and needs a GPU.
> The two share no code.

It is **not a trust anchor**: a receipt verifies against `METHOD_ID` whoever coordinated it, so losing a
coordinator loses the work queue and the scoreboard, never the validity of anything proved. Anyone can stand
up another and provers can point at it. Duplicate work between coordinators is waste, never fault.

libzync will later use the coordinator as the work queue for its whole-chain backfill
([`libzync/ROADMAP.md`](libzync/ROADMAP.md), H-B step (e)).

## 2. API reference

Every endpoint, request and response field, route, status code, coordinator and worker environment
variable (with defaults), worker CLI command and launcher mode is in
[`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md). It is generated from the source by
`python3 scripts/gen-coordinator-reference.py`, and CI (`.github/workflows/adversarial.yml`, `--check`)
fails when it drifts, so it is not copied here. The sponsorship and bot API fields below come from the
hand-written sponsorship docs.

## 3. Running your own

### 3.1 The cost is the data

The software is one file; the data is not (measured once, 2026-08-02, not re-measured):

| what | size | why |
|---|---|---|
| Bitcoin archive node, `txindex=1` | ~864 GB | the bridge needs arbitrary historical transactions |
| witness bundles | ~73 GB, bridge capped at height 220,000 | what provers receive |

The node grows with the chain and bundles grow steeply with height. A new coordinator either resyncs an
archive node or seeds bundles from a peer; it is not a `docker run`.

⚠ **The bridge will fill a disk.** It runs ahead of demand; without a height cap it fills ~10 hours' worth
and stops. Set the cap deliberately.

### 3.2 Seeding from a peer

```bash
curl -s "https://peer.example/api/witnesses?from=1&count=1000" | tar -x -C bundles/
```

Parameters are `from` and `count` (not `lo`/`hi`); `count` defaults to `BULK_MAX`. Compare the manifest's
`missing` (reported, never silently skipped: a bridge gap is not the end of the chain) with what you
extracted. The response is streamed (one `RANGE_SIZE` chunk is a few hundred MB).

⚠ **Check the transfer completed.** The server speaks HTTP/1.0: without `Content-Length` a truncated
transfer looks complete. `tar -x` checks the end-of-archive marker; a naive `read()` loop does not.

### 3.3 Peers and trustless bootstrap

```bash
PEER_COORDINATORS="https://a.example,https://b.example"
PEER_TTL=300              # seconds to cache a peer's proven/busy set
PEER_SYNC_INTERVAL=300    # seconds between background proof adoptions
```

`peer_proven_heights()` and `peer_busy_heights()` exclude a peer's proven and in-flight heights from
`pick()`; `sync_from_peers()` adopts peers' proofs after downloading and verifying each. An unreachable or
malformed peer contributes nothing and never raises. `/api/vranges` deliberately omits `in_bhash`,
`out_bhash` and `range_work`, so a new coordinator cannot take a peer's word for the frontier: it must run
`verify-any` on each proof. `_frontier_chain()` is a pure function of the verified set, so coordinators
with the same set converge with nothing to agree on. Keep it that way.

### 3.4 Minimum configuration

```bash
HAZYNC_HOST=/usr/local/bin/hazync-host   # canonical host binary the coordinator VERIFIES with
VERIFY_MODE=real                         # default once HAZYNC_HOST is set; stated so it cannot slip
COORD_PORT=8899
COORD_BIND=127.0.0.1                     # behind a reverse proxy; 0.0.0.0 only behind a firewall
TRUSTED_PROXIES=127.0.0.1,::1
COORD_DB=/var/lib/hazync/coordinator.db
COORD_STATE=/var/lib/hazync/coord_state
COORD_PROOFS=/var/lib/hazync/proofs      # otherwise defaults to inside the checkout
COORD_SPINE=/var/lib/hazync/spine        # likewise
HAZYNC_BRIDGE_OUT=/var/lib/hazync/bridge_bundles
RANGE_SIZE=1000
CLAIM_TTL=3600
CLAIM_MAX=86400
```

This is `coordinator/deploy/hazync-coordinator.service` and its drop-ins reduced to what a new box needs.
Easy-to-misread startup failures:

- **Empty `HAZYNC_HOST` means `VERIFY_MODE=mock`** (it is the host binary path, not a node address): every
  proof accepted unverified.
- **No `python3-cryptography` means signatures fail closed.**
- Startup refuses a non-loopback `COORD_BIND` when `VERIFY_MODE` is not `real`, `COORD_ALLOW_MOCK` or
  `COORD_ALLOW_UNSIGNED` is set, or the ed25519 library is missing, unless `COORD_ALLOW_PUBLIC_INSECURE`
  (not for a real board).
- **`/var/lib/hazync` must exist and be writable by the service user**, or SQLite cannot create the DB.

`CLAIM_TTL` recovers work from a prover that dies mid-block: too short and slow provers lose work, too long
and a dead prover's blocks sit idle. A claim that never beats is released after `CLAIM_GRACE` (600 s).

A prover needs **your guest**: `hazync selftest` compares its `METHOD_ID` with your `/api/meta`,
`run-workers.sh` refuses to start on a mismatch, and the coordinator rejects other guests' receipts.
Otherwise it needs only a witness bundle per block. A coordinator on a different guest is a different board.

## 4. Sponsorship

Live since **2026-09-26**: `SPONSOR_OPEN=1`, BTCPay connected, the bot on a timer. Anyone can pay for a block
or range to be proven; the payment funds GPU time, the bot proves it, and the block map shows
**Sponsor: <name>** next to the prover. The name is a coordinator record, never inside a proof (that would
change the program ID). First sponsorship: 2,000 sats over Lightning for block 196,001; the bot rented an
RTX 4090, proved it in 819.6 s and released the pod.

⚠ ONE sponsorship has been proved this way: treat the price ladder as a starting point. For that block the
ladder projected $0.01 of GPU against an actual $0.19 (cost follows the UTXO set; a band median predicts a
single block badly).

### 4.1 Flow

```
requested -> invoiced -> paid -> proving -> proven
          |           \-> underpaid (kept as a donation)   \-> refunded
          \-> expired / cancelled
```

| status | set by |
|---|---|
| `requested` | `POST /api/sponsor` (open, span priced, no payments connected) |
| `invoiced` | `POST /api/sponsor` with BTCPay connected |
| `paid` | poller, at least `min_sats` settled, late or not |
| `underpaid` | poller, invoice ended with less than `min_sats` settled |
| `proving` | sponsor bot, when it starts pods |
| `proven` | `submit()` when every block of the span is covered by verified ranges |
| `expired` | poller, invoice ended with nothing paid |
| `cancelled`, `refunded` | operator (not built) |

A name is public **only** if status is `paid`, `proving` or `proven` **and** `paid_sats >= min_sats`
(`SPONSOR_PUBLIC_SQL`, used by every query returning a name and by the bot queue); a row set `paid` by hand
below its minimum shows nothing. **`underpaid` is kept as a donation** (decided 2026-09-13): no name, blocks
not proven for it; the site says so on the form, the private page and the sponsors page.

### 4.2 The minimum and price ladder (decided 2026-09-14)

Whole dollars per block by height, converted to sats; a sponsor may pay more. It is a deliberate
overestimate (prices stay at about **twice** estimated cost) so a paid block can always be proven.

| blocks | min/block | measured |
|---|---:|---|
| 1-200,000 | $1 | trial: slowest $0.18 (2.0 MB); heaviest 1% ~$0.27 |
| 200,001-400,000 | $2 | trial to 230,000: slowest $0.22 (3.7 MB); heaviest 1% ~$0.57; 230,000-400,000 untrialled |
| 400,001-600,000 | $3 | nothing |
| 600,001-800,000 | $4 | nothing |
| 800,001-1,000,000 | $5 | block 966,256: $1.39 of GPU time across 27 cards (2026-09-10) |
| above 1,000,000 | $6 | nothing; runs to `SPONSOR_BAND_TOP` (10^9); unmined blocks cannot be sponsored |

- **Trial 2026-09-14:** two blocks per old band to 230,000 on one RTX 4090 at $0.74/h: 20 s ($0.004) at
  90,002 to 1,047 s ($0.22) at 220,000, i.e. 248-441 s per MB of bundle. Heaviest single blocks, 8.68 MB at
  197,678 and 16.87 MB at 228,538, cost ~$0.79 and ~$1.53: under price, but not by 2x.
- **Bundles reach 418,268** (measured 2026-09-18), so 200,001-400,000 is serviceable (`waiting: 0`;
  `waiting: 11` at 500,000). Above that no bundles exist: a sponsorship is taken and held but waits, the
  quote's `waiting` and the form say so, and the bot rents nothing for it.
- ⚠ **That gap will not close on its own:** the live bridge runs `HAZYNC_BRIDGE_EMIT_FROM=967500`, so
  418,269-967,499 are walked past with no bundle and (until hazync#347 6.6) no checkpoint; filling them means
  replaying from genesis, ~six days at 533 ms/block (measured at h≈706,000). Quote those bands knowing that.
- **To do:** trial each band above 230,000 once its bundles exist and compare cost, overhead included.

**Setting it.** `SPONSOR_PRICE_BANDS` is JSON `[[lo, hi, usd_per_block], ...]` (inclusive, whole dollars);
unset means the ladder above (`SPONSOR_PRICE_BANDS_DEFAULT`). Set but unparseable, empty, zero, negative,
fractional, starting below 1 or overlapping means **unpriced**; a span with any block outside every band is
unpriced; unpriced spans cannot be sponsored. `min_sats = ceil(min_usd × 100,000,000 / rate)`. Both
`min_usd` and `min_sats` are stored per request, so later price or rate changes do not move a quote.

⚠ **The rate** is BTCPay's store rate (`GET /api/v1/stores/<store>/rates?currencyPair=BTC_USD`), the source
that prices the invoice, so quote and checkout agree; cached `SPONSOR_RATE_TTL` (60 s), last good rate used
for `SPONSOR_RATE_STALE` (10 min), then no rate and requests refused. `SPONSOR_BTC_USD` is only the fallback
with no payments connected; it has **no default**, goes stale with the market, and without it a quote has
no sats and a request is `503`. Preview: `SPONSOR_OPEN=1 SPONSOR_BTC_USD=77400`.

### 4.3 Holds

A sponsorship **holds** its blocks while `status IN ('paid','proving') AND paid_sats >= min_sats` (both
non-NULL; `SPONSOR_HOLD_SQL`).

- Normal workers are never offered a held block: holds are in `coverage_and_held()`, read by `claim()`
  (including the frontier re-offer, #284) and `pick()`.
- **Only the sponsorship's own key can prove a held block:** `submit()` returns `403` for a proof newly
  covering it unless the key is registered for THAT sponsorship in `sponsor_keys`. A fold that only
  re-expresses already-proven held blocks is accepted. Tested adversarially over HTTP on a copy of the live
  DB (2026-09-14): before the rule another key's direct submit was accepted and credited.
- The hold ends when the whole span is covered (`proven`, `proven_at`), or on cancel/refund.
- Only open blocks can be sponsored: `409` if any block is proven, under a live claim, held by another
  sponsorship, or already anchored.
- **Holds never expire**, so they are watched: `/api/state` `blocked` names the sponsorship when the
  frontier's next block is held and sets `needs_attention` after `SPONSOR_HOLD_ALERT` (6 h).
- **An open invoice holds softly** for its payment window: nobody is offered the blocks and no other sponsor
  can take them (`409`), but `submit()` accepts a proof of them from anyone, since an unpaid invoice is free.

### 4.4 Payments (BTCPay)

BTCPay Server (`donate.hazync.org`; see hazync-admin `infrastructure/btcpay-box.md`) takes payment;
connected only when `BTCPAY_URL`, `BTCPAY_STORE_ID` and a readable `BTCPAY_API_KEY_FILE` are set
(`payments_enabled()`).

- **Key:** a Greenfield API key for that one store with only create invoices, view invoices, view store
  settings, in a root-only file named by `BTCPAY_API_KEY_FILE`, never in the unit environment (where
  `systemctl show` would print it).
- **Invoice:** for the pledge, in **sats**, payable `SPONSOR_INVOICE_MINUTES` (30), watched for
  `SPONSOR_INVOICE_WATCH` (3 days) after; checkout returns to `SPONSOR_RETURN_URL#t=<token>`. BTCPay
  failure is `503` with nothing recorded.
- Requests are serialised (`_sponsor_request_lock`), span check included.
- **Unpaid caps:** at most `SPONSOR_UNPAID_HOLD_MAX` (5,000) blocks under unpaid invoices and
  `SPONSOR_OPEN_INVOICES_PER_IP` (3) per address (set by the handler, never the body); over either, `429`.
- **Poller** `sponsor_payments_poll`, every `SPONSOR_POLL_S` (30 s), checks `invoiced`, `expired` and
  `underpaid` rows inside their watch window. Only *Settled* payments count:

  | BTCPay reports | becomes |
  |---|---|
  | settled ≥ `min_sats`, any time | `paid` (late payments credited, decided 2026-09-17); straight to `proven` if others finished the span |
  | seen, unconfirmed | stays `invoiced`, hold extended `SPONSOR_CONFIRMING_HOLD` (2 h), re-armed each poll |
  | expired/invalid, settled below min | `underpaid`; `paid` if the rest arrives while watched |
  | expired/invalid, nothing | `expired`; `paid` if payment arrives while watched |
  | marked by hand in BTCPay | left alone and logged; operator sets status |

  Transitions are guarded by their source status, so repeats change nothing. Each payment is recorded in
  `sponsor_payments` (`txid-vout` or Lightning hash, sats, status).
- **Not built:** refunds, a webhook (polling is enough and needs nothing reachable), the public accounts
  ledger, and moderation (takedown) of sponsor names.

### 4.5 Private link and public list

A request returns a `token` once (`secrets.token_urlsafe(24)`); only its sha256 (`token_hash`) is stored, so
neither the DB nor a leaked copy can open a page. The site shows `/sponsors/#<token>` (after `#`: never in a
server log or referrer); it shows every status, including unpaid and `underpaid`. `GET /api/sponsors` lists
public sponsorships, newest payment first, never with pledge, invoice, note or link.

### 4.6 Sponsorship API

| call | answer |
|---|---|
| `GET /api/sponsor` | `{open, max_blocks, payments, priced, bands: [[lo, hi, usd], ...], btc_usd, name_max: 40}` |
| `GET /api/sponsor/quote?lo=&hi=` | `{lo, hi, blocks, min_usd, min_sats, btc_usd, priced, waiting}` (nulls when unpriced/no rate); `waiting` = blocks with no bundle yet |
| `POST /api/sponsor` `{lo, hi, name, amount_sats}` | `503` closed, unpriced or no price; `400` below minimum (with `min_sats`, `min_usd`). Else `202` with `id, token, status, lo, hi, blocks, name, min_usd, min_sats, pledged_sats, message`; with payments also `checkout_url`, `invoice_expires_at`, status `invoiced`; `503` BTCPay failed, `429` unpaid caps, `409` block awaiting another sponsor's payment |
| `GET /api/sponsor/status/<token>` | `id, lo, hi, blocks, name, status, min_usd, min_sats, pledged_sats, paid_sats, created_at, paid_at, proven_at, proven_blocks, queue_ahead, public` (+ `checkout_url`, `invoice_expires_at` while `invoiced`); `404` unknown |
| `GET /api/sponsors` | `{sponsorships: [{id, name, lo, hi, blocks, status, paid_sats, min_usd, min_sats, paid_at, proven_at, proven_blocks, queue_ahead}], open, priced}` |

Validation (quote and request): `1 <= lo <= hi <= tip`, at most `SPONSOR_MAX_BLOCKS` (1000), not anchored
(`409`). A name of 1-40 code points after whitespace collapse, no control, formatting (so no joined emoji),
surrogate or private-use character; the site's form applies the same rule. `amount_sats` a whole number at
least the minimum. `queue_ahead` counts public `paid` sponsorships paid earlier (`null` unless itself
`paid`); `proven_blocks` counts proven, folded or anchored blocks (same runs as `/api/blockstatus`).

Note: `COORDINATOR_REFERENCE.md` describes `GET /api/sponsor`'s `payments` as "always `false`";
`server.py` returns `payments_enabled()`, so it is `true` with BTCPay connected.

## 5. The sponsor bot

`coordinator/sponsor_bot.py` proves paid sponsorships on rented RunPod GPUs and logs what each block cost.
It runs on the coordinator box as user `hazync-sponsor` with **no access to the coordinator's database**
(#351): it reads and changes the board only through the signed, loopback-only bot API, and keeps its cost
log in its own DB. (Its 2026-09-17 doc said "never run live"; the 2026-09-27 sponsorship doc records it
running on a timer and proving block 196,001, which is current.)

### 5.1 Bot API (`/api/bot/`)

- `GET queue` (holding sponsorships, oldest payment first, each with `todo`), `GET blocks?h=` (up to 2,000
  heights: `covered`, `proofs` by key and time, live `claimed`, `held`), `GET sponsorship/<id>` (name,
  status, `held`).
- `POST key {pubkey, sponsorship_id, handle}`: registers in `sponsor_keys` only for a sponsorship that
  holds, only with the handle its name gives, never for a second sponsorship (`409`); the trial key has
  `sponsorship_id` null and handle `SPONSOR: Hazync trial`. `POST proving {sponsorship_id}`: `paid` to
  `proving` only while it holds (`409`). `POST reconcile`: marks proven every hold whose span is covered.
- **The coordinator decides every change**; the bot never supplies a status or a row.
- **Auth:** bot key `SPONSOR_BOT_KEY_FILE` (default `bot.key` in `SPONSOR_BOT_HOME`), public half in
  `SPONSOR_BOT_PUBKEY_FILE`. Headers `X-Hazync-Bot-Ts`, `X-Hazync-Bot-Nonce`, `X-Hazync-Bot-Sig` over
  `bot_message` (method, path with query, ts, nonce, body hash); ts within `SPONSOR_BOT_SKEW`, nonce once.
  `401` unsigned or stale, `403` wrong key, replay or not loopback, `503` no key configured.
- **Loopback only:** a direct connection from `127.0.0.1`/`::1` with no `X-Forwarded-For`; proxies cannot
  reach it.
- **Coordinator restarts are ridden out:** failed looks are retried; after 10 in a row (`COORD_ERRORS_MAX`)
  the run terminates every pod and exits `6`. Spend is checked every look from the bot's own figures.

### 5.2 Behaviour

- Works holds oldest payment first, heights in order, skipping covered heights and those a live pod has;
  several pods can split a sponsorship. Moves `paid` to `proving` on a pod's first block. Reconciles every
  loop (blocks can arrive by other paths).
- **Never claims:** each pod runs `hazync-worker run <n>`, which submits without claiming, and sends no
  heartbeats, so the board shows those blocks as held, not claimed.
- **Only blocks with a bundle or witness** (checked as `server.bundle_path` does) are queued; none rented
  otherwise; `plan` and the log say how many wait.

**Identities: one key per sponsorship**, handle `SPONSOR: <name>` (one bot key would show every sponsor's
blocks under the last one's name). Stored in `<SPONSOR_BOT_HOME>/identities/<id>/` (`key.hex` raw seed hex,
`handle`; 600 in a 700 dir), trials share `identities/trial/`. Handles are cleaned like `clean_handle`
(printable, no `< > & " '`, trimmed: `O'Brien` -> `SPONSOR: OBrien`); names up to 40 fit the 49 limit. On a
pod identities go to `/root/.hazync-ids/<id>/` with `HAZYNC_HOME` per block; `BUNDLE_DIR` and `WITNESS_DIR`
are shared under `/workspace`. `sponsor_work.pubkey` records the key used.

**Reserved prefix:** the coordinator refuses any handle whose folded form (NFKC, lowercase, letters and
digits, `0`->`o`, `5`->`s`) starts with `sponsor` from a key not in `sponsor_keys` (`pubkey`,
`sponsorship_id`, `handle`, `created_at`), in submit, spine submit, rotate (new key) and claim. ⚠ Not
caught: look-alike letters from other scripts (Cyrillic `о`). Also refused: ordinary handles like
`Sponsorship fan` (none in the 2026-09-13 production copy). The bot refuses to run if its key is not accepted.

### 5.3 Money safety

- Live mode requires `--max-pods`, `--max-usd` (run total) and `--max-usd-per-hour`.
- Spend = each pod's `costPerHr` × wall time to confirmed termination: an estimate, not the invoice; how
  RunPod rounds billing is NOT MEASURED.
- A pod deploys only if live hourly rate + `--pod-price-ceiling` (default $1.00/h) fits `--max-usd-per-hour`;
  a dearer pod is terminated at once. Nothing deploys that could reach `--max-usd`; at it, all pods are
  terminated, exit `3`.
- **Every pod is terminated** on exit, exception, SIGINT/SIGTERM, stall (no new proven block for
  `--stall-min`, default 45), failed boot, no public SSH in 15 min, worker finishing without its blocks,
  sponsorship no longer held, or nothing left. Termination repeats `podTerminate` until RunPod no longer
  lists the pod, and sweeps pods with the run's name prefix. ⚠ An unconfirmed pod is printed loudly, exit
  `4`: **terminate it by hand**. `stop-all` terminates every `hz-sponsor-*` pod.
- ⚠ RunPod's Cloudflare refuses Python's default `Python-urllib` User-Agent (`403`, `error code: 1010`); the
  bot sends its own. After 3 refused deploys in a row it exits `5`.
- Work starts with `setsid -f bash sponsor-run.sh >> sponsor-run.log 2>&1 < /dev/null` so ssh returns; a
  timed-out ssh/scp fails the pod (blocks logged `failed`, handed to the next pod).

**Pods** (same recipe as `hazync-board-fleet/fleet.sh`): `podFindAndDeployOnDemand`, 1 GPU, RTX 4090
falling back to A40 (4090 community hosts often never expose public SSH), image
`runpod/pytorch:2.4.0-py3.11-cuda12.4.1-devel-ubuntu22.04`, port 22, 40 GB disk, named
`hz-sponsor-<run>-<n>`. Boot: download the Latest release's worker, run script and CUDA host; check against
PGP-verified `SHA256SUMS.txt`; GPU smoke `prove-block` (#261); program ID vs `/api/meta`. Heights then run
one after another; progress is read from the coordinator.

**Cost log** (`SPONSOR_BOT_DB`, default `bot.db`): `sponsor_work` (one row per block: sponsorship, height,
pod, GPU, price, times, seconds, estimated dollars, outcome `proven`/`stalled`/`failed`/`cancelled`; seconds
split evenly when several land between looks; a `cancelled`/`failed`/`stalled` row becomes `proven` if
proven under its key before its pod's termination) and `sponsor_pods` (price, lifetime, spend, reason).
`report` divides all pod spend by blocks proven, so boot and idle time count.

### 5.4 Setup on the coordinator box

`hazync-sponsor` has no home directory and no access to `coordinator.db`; everything is under
`SPONSOR_BOT_HOME=/var/lib/hazync/sponsor-bot` (owned by it, mode 700):

```
identities/<id>/key.hex, handle   one per sponsorship; identities/trial/ for trials
ssh/id_ed25519, .pub              pods' SSH key (SPONSOR_BOT_SSH_KEY)
known_hosts                       pods' host keys are not checked
gnupg/                            maintainer's release key
runpod.key                        RunPod API key, 600 (RUNPOD_API_KEY_FILE); never printed, no home fallback
bot.key                           bot API key, 600 (`sponsor_bot.py bot-key`)
bot.db                            cost log
work/run-*/                       manifest, boot script, work lists
```

- ssh/scp use `-F /dev/null -i <key> -o IdentitiesOnly=yes -o UserKnownHostsFile=<SPONSOR_BOT_HOME>/known_hosts
  -o GlobalKnownHostsFile=/dev/null -o StrictHostKeyChecking=no` (fresh pods reuse IPs with new host keys).
- `gpg --verify` uses `GNUPGHOME=<SPONSOR_BOT_HOME>/gnupg` (or `SPONSOR_BOT_GNUPGHOME`). Import once:
  `GNUPGHOME=/var/lib/hazync/sponsor-bot/gnupg gpg --import <key>`, fingerprint
  `777F E81F 8CC0 77FD 3D08 055E 852C 2B31 90F5 B928`. A bad manifest signature stops the bot before renting.
- Optional: `SPONSOR_BOT_RELEASE` (default GitHub Latest), `SPONSOR_BOT_META_URL`
  (`http://127.0.0.1:8899/api/meta`), `SPONSOR_BOT_COORD_URL` (`http://127.0.0.1:8899`, loopback, never a proxy).
- `HAZYNC_BRIDGE_OUT` (and `WITNESS_DIR` if the coordinator sets it) must match the coordinator's unit
  (`/var/lib/hazync/bridge_bundles`), or the bot sees only legacy witnesses and rents nothing above them.
- Measured 2026-09-14: outbound HTTPS to RunPod and GitHub and outbound SSH work; `cryptography` 41.0.7,
  `ssh`, `ssh-keygen`, `gpg` installed.

**Moving a box to the API (#351)** (done on server 1, 2026-09-17):
1. Deploy the coordinator with the bot API (inert until `SPONSOR_BOT_PUBKEY_FILE` is set).
2. As `hazync-sponsor`: `sponsor_bot.py bot-key`; put the public key in a root-owned file (e.g.
   `/etc/hazync/sponsor-bot.pub`), set `SPONSOR_BOT_PUBKEY_FILE`, restart.
3. As root: `sqlite3 /var/lib/hazync/coordinator.db ".backup /tmp/coord-copy.db"`, hand it to
   `hazync-sponsor`, run `sponsor_bot.py import-log /tmp/coord-copy.db` (idempotent), delete the copy.
4. `sponsor_bot.py` (plan) must list the queue: key, loopback and pubkey file agree.
5. `chmod 640` `coordinator.db` and its `-wal`/`-shm`, plus `UMask=0027` on the unit (SQLite recreates them
   each checkpoint, so chmod alone does not hold); drop-ins in `coordinator/deploy/dropins/`. Then
   `sudo -u hazync-sponsor sqlite3 /var/lib/hazync/coordinator.db` is refused "unable to open database file".

### 5.5 Commands

```
python3 sponsor_bot.py                    # plan: queue, blocks left, projected cost; touches nothing
python3 sponsor_bot.py run --live --max-pods 2 --max-usd 20 --max-usd-per-hour 2
python3 sponsor_bot.py trial --blocks 100000,150000-150004 --live --max-pods 1 --max-usd 5 --max-usd-per-hour 1
python3 sponsor_bot.py report             # measured cost per block, by height band
python3 sponsor_bot.py stop-all
python3 sponsor_bot.py bot-key            # create bot.key if missing; print its public half
python3 sponsor_bot.py import-log <db>    # copy sponsor_work/sponsor_pods from a coordinator.db copy
```

Tuning: `--pod-price-ceiling`, `--stall-min`, `--blocks-per-pod` (25), `--tick` (30 s). Without `--live`,
`run` and `trial` print the plan and exit `2`. `trial` proves chosen blocks without a sponsorship; it refuses
blocks already proven, without a bundle, claimed within the last hour, or held, and more than 1,000 blocks.

### 5.6 Tests

`coordinator/test_sponsor_bot.py` runs the real RunPod client against a fake GraphQL server, the real
coordinator handler for the bot API, and a fake pod writing verified proofs into a real coordinator DB at
3,600x time. It covers caps, GPU fallback, price ceiling, every termination path, queue order, `proving`/
`proven`, trial refusals, cost log and report, no-bundle blocks, late-landing blocks and row correction,
matching signed bytes, no coordinator table opened, coordinator restarts, `import-log` once, and a refused
key renting nothing. `coordinator/test_sponsor_bot_api.py` attacks the API (unsigned, wrong key, tampered
body/path/query, replay, stale, proxied, non-loopback, no key, forbidden transitions). Each file's
`--control` removes the protections and must fail. **Not tested:** the real RunPod API, SSH/boot/real proving
(`SshRunner`), RunPod billing.

## Related

[`COORDINATOR_REFERENCE.md`](COORDINATOR_REFERENCE.md) · [`OVERVIEW.md`](OVERVIEW.md) · [`OPERATIONS.md`](OPERATIONS.md) (segment coordinator, fleets) ·
[`SECURITY.md`](SECURITY.md) · [`BUILD_AND_RELEASE.md`](BUILD_AND_RELEASE.md) · [`SPEC.md`](SPEC.md) (design
history: `history/HAZYNC_ARCHITECTURE.md`) · hazync#69 (federation design) · `coordinator/deploy/RUNBOOK.md`
