# Core's libsecp256k1 v0.5.1 on OpenVM 2.1 — CPU executor, block 976,379 (23.11.2026)

Stock libsecp v0.5.1 (642c885), same riscv32 gcc 13.2 + flags as Hazync's risc0 guest, plus two hooks inserted by
`ovm-secp/apply_hook.py` at Hazync's own call sites (patches/libzync/0102 kinds 2 and 1; 0007-style liftx):
- **batch**: libsecp parses + range-checks; (e, s, Q, r) / (s, -e, P, r_x) deferred; host hint R; per-check acceptance
  test on R; ONE weighted MSM (128-bit z from a transcript, keys merged) on OpenVM's ECC extension.
- **liftx**: only `secp256k1_fe_sqrt` in `secp256k1_ge_set_xo_var` goes to OpenVM's (hinted, checked) sqrt.

| variant | ECDSA x7,060 cost | BIP340 x899 cost | both, one batch (7,959) | instructions (both) |
|---|---|---|---|---|
| k256 openvm guest-lib (run 1; proved in 130.5 s on A6000) | 8.77B | n/a | n/a | 175M (ECDSA) |
| stock libsecp, software | 529.9B | 67.8B | 597.7B | 15.48B |
| libsecp + batch | 61.5B | 8.3B | 69.4B | 1.78B |
| **libsecp + batch + liftx** | **7.64B** | **1.46B** | **8.67B** | 215M |

Every signature checked; batch: deferred 7,959, distinct keys 3,438, fallback 0.
Negative controls (batch+liftx, mode 5): (a) one hint with y negated, (b) one ECDSA message bit flipped —
both REJECTED by the batch equation (`batch equation failed`, exit 1).
"Cost" is OpenVM's metered cost; proving time not yet measured for these guests (k256 ratio: 8.77B -> 130.5 s).
