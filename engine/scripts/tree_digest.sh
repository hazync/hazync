#!/bin/bash
# tree_digest.sh <dir>: one SHA-256 over a source tree: `sha256sum` of every regular file (paths relative to <dir>,
# sorted bytewise, LC_ALL=C), then the SHA-256 of that list. A build that stages a tree it did not fetch itself (the
# patched OpenVM in the wasm build container) recomputes this and refuses on any difference.
set -euo pipefail
cd "$1"
find . -type f -not -path './.cache/*' -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum | cut -d' ' -f1
