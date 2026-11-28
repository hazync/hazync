# Review round 3 (started 28.11.2026; the user: "do review round 3 whenever you want")

Six fresh reviewers, read-only, at commit a9b78510 (a detached tree, so work landing during the round cannot move
under them), given the code and the question, not round 1-2's conclusions: 1 OpenVM's circuit-level constraints that
every earlier report assumed (the secp256k1 / modular chips: add_ne, double, canonicality, on-curve, hinted sqrt and
non-QR; the deferral circuit's fixed child recursion key; the SHA-256 chip; the buses); 2 the guest's transaction
rules (CheckTxInputs across chunks, BIP68 / MTP, locktime, sigops and weight, coinbase, rv32 arithmetic); 3 the state
commitments (coin log shape proof, BIP30 SMT, the counting trie's passes and combine); 4 the hints and Utreexo zAPPs
(Deriver logic, tip mode); 5 the assume and deposit zAPPs (ECMH, the header-chain composition, inclusion); 6 the newest
code along the trust path (the zero-confirmation guest and its wasm, key and pin handling everywhere, the release and
signing path, the Windows app, the board / tip / sponsor engine paths). Reports kept as given (r3_*.md), assessed in
ASSESSMENT.md before anything is fixed.
