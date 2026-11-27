//! engine/guest/batch_ecdsa_ovm.rs and msm_ovm.rs compiled natively (OpenVM's k256 / ecc / algebra guest crates have
//! host implementations), with probes inside the modules: review R1-3's negatives, run on the current files
//! (`cargo test -- --test-threads=1`) and on the files before the fixes (`cargo test --features old -- --test-threads=1`).
//! The batch module keeps its state in statics, as in the guest: run the tests on one thread.
extern crate alloc;
pub mod batch {
    include!(concat!(env!("OUT_DIR"), "/batch.rs"));
}
