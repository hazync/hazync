#!/bin/bash
# install.sh <host_lz binary> on hazync-proof: the engine prep's dirs, scripts and units (engine/STAGE3.md 3.3). Makes
# NEW paths only; never touches libzync-prep / libzync-board-prep or their dirs. Starts nothing: `systemctl enable --now
# hazync-engine-fetch hazync-engine-prep` after. Stop: `systemctl disable --now hazync-engine-prep hazync-engine-fetch`
# (prep resumes from its newest save point on the next start).
set -euo pipefail
BIN=$(readlink -f "${1:?usage: install.sh <host_lz>}")
HERE=$(cd "$(dirname "$0")" && pwd); REPO=$(cd "$HERE/../../.." && pwd)
R=/srv/bulk/hazync-engine-prep; F=/var/lib/hazync-engine-prep
for d in "$R" "$F"; do [ ! -e "$d" ] || { echo "$d exists: refusing (an install is for a fresh run)" >&2; exit 1; }; done
mkdir -p "$R/bin" "$R/blocks" "$R/leaves" "$F/cl" "$F/trie" "$F/out"
for d in cl trie out; do ln -s "$F/$d" "$R/$d"; done
install -m 0755 "$BIN" "$R/bin/host_lz"
install -m 0755 "$REPO/coordinator/deploy/libzync/libzync-prep-run.sh" "$R/bin/libzync-prep-run.sh"
install -m 0644 "$REPO/prover/fetch_block_rpc.py" "$R/bin/fetch_block_rpc.py"
install -m 0644 "$HERE/hazync-engine-prep.service" "$HERE/hazync-engine-fetch.service" /etc/systemd/system/
systemctl daemon-reload
sha256sum "$R/bin/"* /etc/systemd/system/hazync-engine-*.service
