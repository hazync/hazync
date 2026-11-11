# Header chain at the tip (hazync.org/headers/)

`hazync-headers-follow` keeps the header-chain zyncAPP proof at Bitcoin's tip. The page reads two receipts:

- **the base**, genesis..B, proved once on a GPU (`experimental/libzync-bench/headers/run_headers_pod.sh`, ~57 min,
  ~$2.40 on one RTX PRO 6000);
- **one run**, B+1..tip, chained to the base with `--after`, re-proved on the CPU every time the tip moves.

The page never reads more than two receipts, and a reorg above B needs nothing special: the next run follows the new
tip. When the run gets long (a warning at 20,000 headers), prove a new base and install it with `--set-base`.

A run is published only after `swiftsync-verify headers --expect <tip>:<the node's hash>` prints
`SWIFTSYNC_VERIFY_OK` over base + run. It proves only with the published ELF (`5bd8011d`, image `2db34e12`).

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

## Upgrading the image (risc0 3.1.0, 11-11)

The headers image moved from `4d04d6bf` to `2db34e12`. The installed base (proved by `4d04d6bf`) no longer
verifies, so every timer run fails until a base of the new image is installed. Install the new binary, then
re-run the `--set-base` line above with the 3.1.0 `headers_0_tip.rcpt` (`experimental/libzync-bench/headers/`,
sha256 `a2931d89…`).

## Test

`test/test_follow.sh <zyncapp> <swiftsync-verify> <elf> <base receipt> <headers files>` runs the script against a fake
bitcoin-cli serving real mainnet headers, with the real prover and verifier (two CPU proofs, ~1.5 min): base install
(a wrong hash refused), publish, unchanged tip, a moved tip replacing the run, and three refusals that leave the
published folder untouched (a reorg below the base, a run that does not verify, a wrong ELF). Control: with the
verify step removed, the "does not verify" case fails.
