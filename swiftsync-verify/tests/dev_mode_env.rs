//! RISC0_DEV_MODE in the verifier's environment (review 11-18): risc0 built with disable-dev-mode PANICS in
//! Receipt::verify when it is 1 / true / yes, an abort under panic = "abort" (the C ABI's host). verify() must
//! refuse before calling it (the header-chain zyncAPP has its own: zapps/headers/tests/dev_mode_env.rs). Its own test binary: the variable is process-wide.
use swiftsync_verify::{verify, Image, Inputs, Verdict};

const RCPT: &[u8] = include_bytes!("../vectors/mainnet_1_200_8888d59c.rcpt");

#[test]
fn dev_mode_is_a_fail_not_a_panic() {
    let image = Image::by_id("mainnet", "8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4").unwrap();
    for v in ["1", "true", "yes", "TRUE", "Yes"] {
        std::env::set_var("RISC0_DEV_MODE", v);
        let r = verify(&Inputs { receipt: RCPT, image, anchor: None, now: 1_800_000_000 });
        assert_eq!(r.verdict, Verdict::Fail, "{v}");
        assert!(r.checks.iter().any(|(ok, w)| !ok && w.contains("RISC0_DEV_MODE")), "{v}: {:?}", r.checks);
    }
    std::env::remove_var("RISC0_DEV_MODE");
    assert_eq!(verify(&Inputs { receipt: RCPT, image, anchor: None, now: 1_800_000_000 }).verdict, Verdict::Demo);
}
