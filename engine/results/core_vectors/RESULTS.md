# Core's test vectors on the engine vs RISC Zero (25.11.2026)

All 22,666 of Bitcoin Core v28.0's script and transaction test vectors (scripts/libzync/core_vectors.py over
src/test/data: 17,979 script_tests, 2,741 tx_valid, 1,946 tx_invalid) through guest mode 10 (plain libsecp path),
on RISC Zero and on the engine (hz24 keys), verdict for verdict.

- `host core-vectors-export` (prover/host, ee16f155): batches of 500 as the exact input streams RISC Zero's guest
  reads, with RISC Zero's verdict journal for each; a batch RISC Zero aborts is halved until each abort is alone.
  Result: 21,987 cases with a RISC Zero verdict in 176 streams, 679 lone RISC Zero aborts (Core's script-number
  exceptions, which the guest cannot unwind; Core itself rejects those cases), 855 streams (cv_export.log).
- `cv_drive.py`: every stream executed on the engine (hzengine serve, keys loaded once), each verdict journal
  compared byte for byte.

**Result (cv.out): 176/176 streams with IDENTICAL verdicts (21,987 cases), 0 differing; all 679 RISC Zero aborts
also refused by the engine, 0 accepted; 0 engine-only aborts.** The engine agrees with RISC Zero, and so with Core's
expectations as the product's core-vectors run checks them, on every vector.
