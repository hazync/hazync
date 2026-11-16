#!/bin/bash
# wasm-image-tag.sh <rust toolchain> <with C: 0|1>: the tag of the WASM build image (reproduce/wasm.Dockerfile built with
# those build args), 16 hex digits of sha256(Dockerfile, "<toolchain> <with C>"). One definition for the build scripts'
# container block and .github/workflows/publish-wasm-image.yml, so a published tag is the one the scripts look up.
set -euo pipefail
[ $# = 2 ] && [[ $2 =~ ^[01]$ ]] || { echo "usage: $0 <rust toolchain> <0|1>" >&2; exit 1; }
cat "$(dirname "$(realpath "${BASH_SOURCE[0]}")")/wasm.Dockerfile" <(echo "$1 $2") | sha256sum | cut -c1-16
