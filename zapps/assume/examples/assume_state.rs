//! `cargo run --release --example assume_state -- <state.json> <headers.bin> <tracked.txt|-> <snapshot>`
//!
//! TESTING ONLY: the assumeutxo check of `zapp-assume assumeutxo`, against a ChainState JSON (a prover run's
//! `state_<h>.json`, the host's native state) instead of a verified receipt. It tests the snapshot encoding at heights
//! no proof in the repo reaches (BIP30-tracked coins, the 91,842 / 91,880 overwrites) and measures the hashing; it
//! proves nothing about the chain. Prints UNPROVEN-STATE-MATCH (exit 0) or UNPROVEN-STATE-MISMATCH (exit 1).
use zapp_assume as assume;
use swiftsync_verify::ChainState;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [state, headers, tracked, snap] = a.as_slice() else {
        eprintln!("usage: assume_state <state.json> <headers.bin> <tracked.txt|-> <snapshot>");
        std::process::exit(1)
    };
    let cs: ChainState = serde_json::from_slice(&std::fs::read(state).unwrap()).unwrap();
    let hc = assume::HeaderChain::bind(&std::fs::read(headers).unwrap(), &cs.tip_hash, cs.height).unwrap();
    let t = if tracked == "-" { assume::Tracked::new() } else { assume::parse_tracked(&std::fs::read_to_string(tracked).unwrap()).unwrap() };
    let t0 = std::time::Instant::now();
    let f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(snap).unwrap());
    let r = assume::check_snapshot(&cs, f, &hc, &t);
    let s = t0.elapsed().as_secs_f64();
    match r {
        Ok(r) => {
            println!("UNPROVEN-STATE-MATCH height {} coins {} value {} tracked {} of {} listed; hash_serialized_3 {}; {:.2} s, {:.2} us/coin",
                     cs.height, r.coins, r.value, r.tracked_used, t.len(), r.hash_serialized_3, s, s * 1e6 / r.coins.max(1) as f64);
        }
        Err(e) => {
            println!("UNPROVEN-STATE-MISMATCH height {}: {e} ({s:.2} s)", cs.height);
            std::process::exit(1)
        }
    }
}
