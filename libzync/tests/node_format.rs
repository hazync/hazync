//! The coin log's node formats (COINLOG.md §2). Its own test binary: the format is per process (coinlog::set_node_format),
//! and unit tests run in parallel threads of one process.
use libzync::coinlog::{node_format, parent, set_node_format, spender, spent, use_format_of, PROFILE_NODE2_BIT, H};
use sha2::{Digest, Sha256};

fn hx(h: H) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn both_formats_against_the_independent_reference() {
    let x: H = Sha256::digest(b"c").into();
    let y: H = Sha256::digest(b"d").into();
    // format 2, the default (scripts/libzync/coinlog_ref.py, which shares no code with libzync)
    assert_eq!(node_format(), 2);
    assert_eq!(hx(spent(&x, &spender(974000, 5, 1))), "da3dcccdbe802c5f5c22804744b293de00277d2e8d93ab520741be733e9988b3");
    assert_eq!(hx(parent(3, &x, &y)), "22a1d6fc91fcb8309d0334439ae43bfe16fe9697e0153c19531ae65d4ff1296c");
    // format 1, the chains proved before 26.11.2026: the values the reference gave before its switch, so a fact about
    // an old chain still holds once a verifier reads that chain's profile
    assert_eq!(use_format_of(0x71), 1);
    assert_eq!(hx(spent(&x, &spender(974000, 5, 1))), "cdcc3dda5f2c80be4780225cde93b2442e7d270052ab938d73be4107b388993e");
    assert_eq!(hx(parent(3, &x, &y)), "fcd6a12230b8fc914334039dfe3be49a9796fe16193c15e05de61a536c29f14f");
    // and back, by profile
    assert_eq!(use_format_of(0x71 | PROFILE_NODE2_BIT), 2);
    assert_eq!(hx(parent(3, &x, &y)), "22a1d6fc91fcb8309d0334439ae43bfe16fe9697e0153c19531ae65d4ff1296c");
    set_node_format(2);
}

#[test]
fn the_profile_bit_is_facts_bit() {
    assert_eq!(PROFILE_NODE2_BIT, libzync::facts::PROFILE_NODE2);
}
