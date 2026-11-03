//! `zync-path`: the path producer for zyncAPP coin facts (docs/libzync/ZYNCAPPS.md §5). Reads a proving run's
//! `history.bin` (libzync::history) and writes a [`swiftsync_verify::CoinFact`] that `swiftsync-verify --coin` checks
//! against the receipt at `<tip>`. It holds no secrets and proves nothing by itself: a wrong path simply fails.
//!
//!   zync-path coin <history.bin> <tip> <at> <position>      the coin at a log position, its state after height <at>
//!   zync-path find <history.bin> <tip> <coin leaf hex>       the log position of a coin leaf
//!
//! ⚠ Replays the whole history in memory (C1's disk-backed store replaces this for mainnet).

use libzync::history::{block_path, decode_all, replay};
use swiftsync_verify::{hex, hex32, CoinFact};

fn die(m: &str) -> ! {
    eprintln!("zync-path: {m}");
    std::process::exit(1);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize| -> u64 { a.get(i).and_then(|x| x.parse().ok()).unwrap_or_else(|| die("usage: see the source header")) };
    let hist = || decode_all(&std::fs::read(a.get(1).unwrap_or_else(|| die("history.bin?"))).unwrap_or_else(|e| die(&e.to_string())))
        .unwrap_or_else(|e| die(e));
    match a.first().map(|s| s.as_str()) {
        Some("coin") => {
            let (tip, at, pos) = (num(2) as u32, num(3) as u32, num(4));
            let r = replay(&hist(), at, tip).unwrap_or_else(|e| die(e));
            if pos >= r.log.len() {
                die(&format!("position {pos}: the log after {at} has {} coins", r.log.len()));
            }
            let block = (at < tip).then(|| {
                let (path, _) = block_path(&r.block_leaves, at as u64);
                let recs = hist();
                (hex(&recs[at as usize].hash), path.iter().map(|x| hex(x)).collect())
            });
            let f = CoinFact {
                at, pos,
                coin: hex(&r.coins[pos as usize]),
                spent_by: r.spenders.get(&pos).copied(),
                path: r.log.path(pos).iter().map(|x| hex(x)).collect(),
                log_leaves: r.log.len(),
                log_value: r.value,
                log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(),
                block,
            };
            println!("{}", serde_json::to_string_pretty(&f).unwrap());
        }
        Some("find") => {
            let tip = num(2) as u32;
            let want = hex32(&a.get(3).unwrap_or_else(|| die("coin leaf?")).to_lowercase()).unwrap_or_else(|| die("coin leaf: 64 hex digits"));
            let r = replay(&hist(), tip, tip).unwrap_or_else(|e| die(e));
            match r.coins.iter().position(|c| *c == want) {
                Some(p) => println!("{p}"),
                None => die("no such coin leaf in the log (coins created and spent in one block never enter it)"),
            }
        }
        _ => die("usage: zync-path coin <history.bin> <tip> <at> <pos> | zync-path find <history.bin> <tip> <coin leaf hex>"),
    }
}
