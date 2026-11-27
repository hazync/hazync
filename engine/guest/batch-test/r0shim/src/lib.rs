//! The one guest API item batch_ecdsa_ovm.rs uses (env::log), on the host.
pub mod guest {
    pub mod env {
        pub fn log(s: &str) { eprintln!("{s}"); }
    }
}
