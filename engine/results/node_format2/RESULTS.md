# Coin log / trie node format 2 (user decision 26.11.2026; commit 6fd07c19)

The node hash is the same SHA-256 compression over the same 64 bytes; format 2 keeps the result's 8 state words as they
are (little-endian bytes) where format 1 byte-swapped each into FIPS 180-4's digest order (docs/libzync/COINLOG.md §2).
The restart from genesis makes every root new, so nothing has to stay compatible; the existing chain's proofs still
verify (ChainState profile bit 7, PROFILE_NODE2, names the format; hosts and verifiers dispatch per thread).

**Tests** (laptop): libzync 63, swiftsync-verify 33, zAPPs assume 31, common 15, corecheck 6, deposit 23, headers 17,
hints 2, liveness 17, statement 20, utreexo 13: all pass (240). Includes known-answer vectors for BOTH formats from two
independent Python implementations (a fresh SHA-256 compression checked against hashlib, and scripts/libzync/
coinlog_ref.py), a real proof of the existing RISC Zero chain (mainnet 1..200, image 8888d59c) whose trie fact still
verifies in format 1, and old-chain liveness fixtures.

**Both provers, one block** (build box, fmt2.sh): the product host and its RISC Zero guest (METHOD_ID b10e1728...) and
the engine guest (exe b73f1d13..., vm 00e00077) built from the same commit, block 976,379 exported:
- every chunk journal (5) and the trie journal IDENTICAL between the engine and RISC Zero;
- the aggregate: 32 of 5,284 bytes differ, the image id it carries (as on every block): the chain state is the same.
- The trie update's cost on the engine: **4.84B, 52.9M instructions** (format 1, same block: 5.50B, 66.5M): **-12% cost,
  -20% instructions**. (The earlier bench estimate was -22%; this is the measured figure on the real trie.)
