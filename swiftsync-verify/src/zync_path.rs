//! `zync-path`: the path producer for zyncAPP coin facts (docs/libzync/ZYNCAPPS.md §8). Reads a proving run's
//! `history.bin` (libzync::history) and writes a [`swiftsync_verify::CoinFact`] that `swiftsync-verify --coin` checks
//! against the receipt at `<tip>`. It holds no secrets and proves nothing by itself: a wrong path simply fails.
//!
//!   zync-path coin <history.bin> <tip> <at> <position>      the coin at a log position, its state after height <at>
//!   zync-path find <history.bin> <tip> <coin leaf hex>       the log position of a coin leaf
//!   zync-path journey <history.bin> <tip> <position> --bitcoin-cli "<cmd>"
//!                                                            created-by tx, state at the tip, spent-by tx
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
        Some("journey") => journey(&a),
        _ => die("usage: zync-path coin <history.bin> <tip> <at> <pos> | zync-path find <history.bin> <tip> <coin leaf hex>"),
    }
}

/// `<cmd> <args>` on the user's node; stdout trimmed.
fn node(cmd: &str, args: &[&str]) -> String {
    let mut p = cmd.split_whitespace();
    let prog = p.next().unwrap_or_else(|| die("empty --bitcoin-cli"));
    let o = std::process::Command::new(prog).args(p).args(args).output().unwrap_or_else(|e| die(&format!("{prog}: {e}")));
    if !o.status.success() {
        die(&format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn unhex(s: &str) -> Vec<u8> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        die("node returned non-hex");
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or_else(|_| die("node returned non-hex"))).collect()
}

/// Block `h` from the node: (raw hex, header, txs). Checked against the history's hash for that height.
fn block_at(cli: &str, h: u32, want: &[u8; 32]) -> (Vec<u8>, [u8; 80], Vec<libzync::facts::TxView>) {
    let hash = node(cli, &["getblockhash", &h.to_string()]);
    let raw = unhex(&node(cli, &["getblock", &hash, "0"]));
    let (header, txs) = libzync::facts::parse_block(&raw).unwrap_or_else(|| die(&format!("block {h}: cannot parse")));
    if &libzync::facts::header_hash(&header) != want {
        die(&format!("block {h}: the node's block is not the proven chain's (history hash differs)"));
    }
    (raw, header, txs)
}

fn tx_fact(r: &libzync::history::Replay, h: u32, header: &[u8; 80], txs: &[libzync::facts::TxView], index: usize) -> swiftsync_verify::TxFact {
    let ids: Vec<[u8; 32]> = txs.iter().map(libzync::facts::txid).collect();
    swiftsync_verify::TxFact {
        height: h,
        header: hex(header),
        state: hex(&r.states[h as usize]),
        block_path: block_path(&r.block_leaves, h as u64).0.iter().map(|x| hex(x)).collect(),
        tx: hex(&txs[index].stripped),
        index: index as u32,
        branch: libzync::facts::merkle_branch(&ids, index).iter().map(|x| hex(x)).collect(),
        coinbase: hex(&txs[0].stripped),
        coinbase_branch: libzync::facts::merkle_branch(&ids, 0).iter().map(|x| hex(x)).collect(),
    }
}

/// `zync-path journey <history.bin> <tip> <position> --bitcoin-cli "<cmd>"`: the coin's creating transaction, its
/// state at the tip, and its spending transaction if any.
pub fn journey(a: &[String]) {
    let cli = a.iter().position(|x| x == "--bitcoin-cli").and_then(|i| a.get(i + 1)).unwrap_or_else(|| die("journey needs --bitcoin-cli \"<cmd>\""));
    let recs = decode_all(&std::fs::read(&a[1]).unwrap_or_else(|e| die(&e.to_string()))).unwrap_or_else(|e| die(e));
    let tip: u32 = a[2].parse().unwrap_or_else(|_| die("tip?"));
    let pos: u64 = a[3].parse().unwrap_or_else(|_| die("position?"));
    let r = replay(&recs, tip, tip).unwrap_or_else(|e| die(e));
    let coin = *r.coins.get(pos as usize).unwrap_or_else(|| die("no such position"));
    // the height that appended `pos`
    let mut n = 0u64;
    let oh = recs[..=tip as usize].iter().find(|x| { n += x.appends.len() as u64; n > pos }).map(|x| x.height).unwrap();
    let (_, header, txs) = block_at(cli, oh, &recs[oh as usize].hash);
    let mtp: u32 = if oh == 0 { 0 } else {
        let ph = node(cli, &["getblockhash", &(oh - 1).to_string()]);
        let j: serde_json::Value = serde_json::from_str(&node(cli, &["getblockheader", &ph])).unwrap_or_else(|e| die(&e.to_string()));
        j["mediantime"].as_u64().unwrap_or_else(|| die("no mediantime")) as u32
    };
    let mut found = None;
    'f: for (i, t) in txs.iter().enumerate() {
        let id = libzync::facts::txid(t);
        for (v, (value, spk)) in t.outputs.iter().enumerate() {
            for tracked in [false, true] {
                if libzync::facts::coin_leaf(&id, v as u32, *value, spk, oh, i == 0, tracked, mtp) == coin {
                    found = Some((i, v as u32, tracked));
                    break 'f;
                }
            }
        }
    }
    let (i, vout, tracked) = found.unwrap_or_else(|| die(&format!("no output of block {oh} has this coin's leaf")));
    let spent_by = r.spenders.get(&pos).copied();
    let spend = spent_by.map(|(h, t, _)| {
        let (_, sh, stx) = block_at(cli, h, &recs[h as usize].hash);
        tx_fact(&r, h, &sh, &stx, t as usize)
    });
    let j = swiftsync_verify::CoinJourney {
        origin: tx_fact(&r, oh, &header, &txs, i),
        vout, tracked, creation_mtp: mtp,
        fact: CoinFact {
            at: tip, pos, coin: hex(&coin), spent_by,
            path: r.log.path(pos).iter().map(|x| hex(x)).collect(),
            log_leaves: r.log.len(), log_value: r.value,
            log_roots: r.log.roots().iter().map(|x| x.map(|h| hex(&h))).collect(),
            block: None,
        },
        spend,
    };
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}
