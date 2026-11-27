RISC Zero receipts from swiftsync-verify/vectors as they were before the cutover (engine/CUTOVER.md C2, commit
2e9463f2 removed them there): the RISC Zero coordinator's upload check (src/lz_coord.rs `verify_any`) is tested on
them until that code goes with Stage 3. See the old swiftsync-verify/vectors/README.md (git show 2e9463f2^:...) for
how each was made.
