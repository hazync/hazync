//! RISC0_DEV_MODE in the verifier's environment (review 11-18, carried here from swiftsync-verify when #758 met the
//! move of the header-chain reader out of the core, #724): risc0 built with disable-dev-mode PANICS in Receipt::verify
//! when it is 1 / true / yes, an abort under panic = "abort". verify_headers() must refuse before calling it. Its own
//! test binary: the variable is process-wide.
use swiftsync_verify::Verdict;
use zapp_headers::{verify_headers, HEADERS_IMAGE_ID};

const HDR_0_100: &[u8] = include_bytes!("../../../experimental/libzync-bench/headers/hdr_0_100.rcpt");

#[test]
fn dev_mode_is_a_fail_not_a_panic() {
    for v in ["1", "true", "yes", "TRUE", "Yes"] {
        std::env::set_var("RISC0_DEV_MODE", v);
        let h = verify_headers(&[HDR_0_100], HEADERS_IMAGE_ID, 1_800_000_000, None, None);
        assert_eq!(h.verdict, Verdict::Fail, "{v}");
        assert!(h.checks.iter().any(|(ok, w)| !ok && w.contains("RISC0_DEV_MODE")), "{v}: {:?}", h.checks);
    }
    std::env::remove_var("RISC0_DEV_MODE");
    let h = verify_headers(&[HDR_0_100], HEADERS_IMAGE_ID, 1_800_000_000, None, None);
    assert!(h.checks.iter().any(|(ok, w)| *ok && w.contains("RISC0_DEV_MODE is not set")), "{:?}", h.checks);
}
