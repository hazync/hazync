# Header chain at the tip (hazync.org/headers/)

`hazync-headers-follow` keeps the header-chain zyncAPP proof at Bitcoin's tip. The page reads two receipts:

- **the base**, genesis..B, proved once on a GPU (`experimental/libzync-bench/headers/run_headers_pod.sh`; the
  3.1.0 base, genesis..975,278, took 59.4 min, ~$2.50, on one RTX PRO 6000);
- **one run**, B+1..tip, chained to the base with `--after`, re-proved on the CPU every time the tip moves.

The page never reads more than two receipts, and a reorg above B needs nothing special: the next run follows the new
tip. When the run gets long (the script warns at 20,000 headers, `REBASE_WARN`), prove a new base and install it with
`--set-base`. Memory decides "long" well before that warning (next section).

## Memory: re-base before the run grows

A run of N headers re-proves **all N** every time the timer fires, so both its time and its memory grow with the
distance from the base. Measured on hazync-proof: at **250 headers** the run peaked at **9.8 GB** of RAM. The unit's
old `MemoryMax=6G` OOM-killed every run from 2026-11-11 19:10 until it was raised; the box now runs it with
`MemoryMax=24G`. ⚠ **The `hazync-headers-follow.service` in this directory still says `MemoryMax=6G`**: on a fresh
install raise it (a drop-in or an edit) to well above 6G, or the first long run is killed.

Do not let the run grow unbounded: prove a fresh base on a GPU (`run_headers_pod.sh`) and install it with
`--set-base` as the run grows. How far a run can grow before 24G is not enough has not been measured.

A run is published only after `swiftsync-verify headers --expect <tip>:<the node's hash>` prints
`SWIFTSYNC_VERIFY_OK` over base + run. It proves only with the published ELF (`5bd8011d`, image `2db34e12`).

## Block inclusion (the MMR)

Since the MMR image (feat/headers-mmr) the state also carries the peaks of a Merkle Mountain Range of every block
hash (`prover/zyncapps/methods/headers/src/mmr.rs`), so the published pair answers "is block X at height h on this
chain": `zyncapp headers-mmr-path <headers file from 0> <h> --tip <tip> > path.txt`, then
`swiftsync-verify headers --include <h>:<hash> --path path.txt <base> <run>` (add `--tx <txid>:<pos> --header <hex80>
--branch <file>` for a transaction in that block). The follower needs no change: `--after` reads the base's peaks
from its journal. A base from an older image has no peaks: the first MMR base is re-proved from genesis.

## Where it runs

- **hazync-proof** proves, with its own bitcoind (`bitcoin-cli -conf=/etc/bitcoin/bitcoin.conf -datadir=/var/lib/bitcoind`, as root with the cookie),
  at the lowest CPU and I/O priority. Measured: 37 headers in 49 s on a laptop CPU.
- **The web box** serves `/var/www/hazync-headers/` at `/headers-live/` (nginx alias, outside the site root, so the
  site deploy's `rsync --delete` cannot touch it). The publish key is pinned there to
  `rrsync -wo /var/www/hazync-headers`.

## Install (hazync-proof)

```sh
install -d /opt/hazync-headers/bin /var/lib/hazync-headers
install -m755 hazync-headers-follow /opt/hazync-headers/bin/
# zyncapp + swiftsync-verify from this commit (release builds), and the container-built ELF:
install -m755 zyncapp swiftsync-verify /opt/hazync-headers/bin/
install -m644 zyncapp-headers.bin /opt/hazync-headers/bin/      # sha256 5bd8011d…
ssh-keygen -t ed25519 -N '' -C hazync-headers-publish -f /root/.ssh/hazync_headers_publish
#   web box, /root/.ssh/authorized_keys:
#   command="/usr/bin/rrsync -wo /var/www/hazync-headers",restrict ssh-ed25519 AAAA… hazync-headers-publish
HEADERS_DIR=/var/lib/hazync-headers HEADERS_BIN=/opt/hazync-headers/bin \
  /opt/hazync-headers/bin/hazync-headers-follow --set-base headers_0_tip.rcpt 975278 00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3
cp hazync-headers-follow.service hazync-headers-follow.timer /etc/systemd/system/
systemctl daemon-reload && systemctl enable --now hazync-headers-follow.timer
```

## Upgrading the image (risc0 3.1.0, done 2026-11-12)

The headers image moved from `4d04d6bf` to `2db34e12`. The installed base (proved by `4d04d6bf`) no longer
verifies, so every timer run fails until a base of the new image is installed. Re-run the four `install`
lines above with this release's files: `hazync-headers-follow` (its `WANT_ELF` is now `5bd8011d…`, so the old
script refuses the new ELF), `zyncapp`, `swiftsync-verify` and `zyncapp-headers.bin`. Then re-run the `--set-base` line with the 3.1.0 `headers_0_tip.rcpt` (`experimental/libzync-bench/headers/`,
sha256 `a2931d89…`).

As done on hazync-proof on 2026-11-12: `zyncapp` was built from the release commit with `RISC0_SKIP_BUILD=1` (the guest
build is skipped; the ELF is supplied separately), the ELF installed was the container-built `zyncapp-headers.bin`
(sha256 `5bd8011d…`, image `2db34e12`), and the base installed was the 3.1.0 `headers_0_tip.rcpt` (sha256
`a2931d89…`, genesis..975,278) with `--set-base headers_0_tip.rcpt 975278 <hash of 975,278>`. The base proof took
59.4 min on one RTX PRO 6000 (~$2.50) and is 224 KB.

## Test

`test/test_follow.sh <zyncapp> <swiftsync-verify> <elf> <base receipt> <headers files>` runs the script against a fake
bitcoin-cli serving real mainnet headers, with the real prover and verifier (two CPU proofs, ~1.5 min): base install
(a wrong hash refused), publish, unchanged tip, a moved tip replacing the run, and three refusals that leave the
published folder untouched (a reorg below the base, a run that does not verify, a wrong ELF). Control: with the
verify step removed, the "does not verify" case fails.
