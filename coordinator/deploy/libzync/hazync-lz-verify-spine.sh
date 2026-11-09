#!/bin/sh
# The libzync board's spine verifier for hazync-offsite-proofs.py spine --verify: exit 0 only when the receipt
# verifies against the libzync image AND is anchored on the genesis state (`verify-any <receipt> 1`: a libzync host
# refuses a receipt whose anchor is not the digest of genesis). One argument, as the CORE hazync-verify takes.
[ $# -eq 1 ] || { echo "usage: hazync-lz-verify-spine <spine receipt>" >&2; exit 2; }
exec "${HAZYNC_HOST:-/opt/hazync-lz/bin/host}" verify-any "$1" 1
