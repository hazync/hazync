# batch-test: the batch signature check, natively (review R1-3)

engine/guest/batch_ecdsa_ovm.rs and msm_ovm.rs compiled on the host (OpenVM's k256 / ecc / algebra guest crates have
host implementations; RustCrypto's k256 builds the test points independently). build.rs inlines the guest files into
one module with probes, so the tests reach the module's private state.

    cargo test --release -- --test-threads=1 --nocapture                  # the current files
    cargo test --release --features old -- --test-threads=1 --nocapture   # the files before the R1-3 fixes (old/, 0382bb3f^)

(a) a forged BIP340 item (r = 0, s = e·d) with the hint R = (0,0); (b) an ECDSA item's R negated: the transcript and the
batch's verdict; (c) the MSM's window size and its sums against a naive sum. Results: ../../reviews/round1/r1_3_verification.md.
