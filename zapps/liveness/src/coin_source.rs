//! Where a liveness producer (`zync-path live`, `zync-serve /live`) learns a coin's fields: the creating block, from
//! block files (`block_<h>.json`, the repo's prover/testdata format) or the user's own node (`bitcoin-cli`). Nothing
//! here is trusted: the producer keeps only a coin whose leaf the proven coin log holds, and the verifier recomputes
//! that leaf from the fields.

use libzync::liveness::Coin;
use std::path::PathBuf;

pub enum Source {
    Dir(PathBuf),
    Cli(String),
}

impl Source {
    /// `--blocks <dir>` or `--bitcoin-cli "<cmd>"` among the arguments.
    pub fn from_args(a: &[String]) -> Option<Source> {
        let flag = |f: &str| a.iter().position(|x| x == f).and_then(|i| a.get(i + 1)).cloned();
        flag("--blocks").map(|d| Source::Dir(d.into())).or_else(|| flag("--bitcoin-cli").map(Source::Cli))
    }
}

fn unhex(s: &str) -> Result<Vec<u8>, String> {
    swiftsync_verify::unhex(s.trim()).ok_or_else(|| "not hex".to_string())
}

/// (height, every transaction, the creating block's parent's median time past) for one block.
type Block = (u32, Vec<libzync::facts::TxView>, u32);

fn median(mut t: Vec<u32>) -> u32 {
    t.sort_unstable();
    t.get(t.len() / 2).copied().unwrap_or(0)
}

fn json_block(p: &std::path::Path) -> Result<Block, String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?)
        .map_err(|e| format!("{}: {e}", p.display()))?;
    let h = v["height"].as_u64().ok_or("block file: no height")? as u32;
    let mut raws = vec![unhex(v["coinbase_hex"].as_str().ok_or("block file: no coinbase_hex")?)?];
    for t in v["txs"].as_array().ok_or("block file: no txs")? {
        raws.push(unhex(t["raw"].as_str().ok_or("block file: a tx without raw")?)?);
    }
    let txs = raws.iter().map(|r| {
        let mut o = 0;
        libzync::facts::parse_tx(r, &mut o).filter(|_| o == r.len()).ok_or_else(|| format!("{}: a transaction does not parse", p.display()))
    }).collect::<Result<Vec<_>, _>>()?;
    // recent_times: the timestamps of the blocks before this one (at most 11), so their median is the parent's MTP
    let times = v["recent_times"].as_array().ok_or("block file: no recent_times")?.iter().filter_map(|x| x.as_u64().map(|t| t as u32)).collect();
    Ok((h, txs, median(times)))
}

fn node(cmd: &str, args: &[&str]) -> Result<String, String> {
    let mut p = cmd.split_whitespace();
    let prog = p.next().ok_or("empty --bitcoin-cli")?;
    let o = std::process::Command::new(prog).args(p).args(args).output().map_err(|e| format!("{prog}: {e}"))?;
    if !o.status.success() {
        return Err(format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn cli_block(cli: &str, h: u32) -> Result<Block, String> {
    let hash = node(cli, &["getblockhash", &h.to_string()])?;
    let raw = unhex(&node(cli, &["getblock", &hash, "0"])?)?;
    let (_, txs) = libzync::facts::parse_block(&raw).ok_or_else(|| format!("block {h}: cannot parse"))?;
    let mtp = if h == 0 { 0 } else {
        let ph = node(cli, &["getblockhash", &(h - 1).to_string()])?;
        let j: serde_json::Value = serde_json::from_str(&node(cli, &["getblockheader", &ph])?).map_err(|e| e.to_string())?;
        j["mediantime"].as_u64().ok_or("no mediantime")? as u32
    };
    Ok((h, txs, mtp))
}

/// Every reading of the coin `txid:vout` (txid internal order) the creating block allows: the BIP30 flag (bit 1 of
/// the leaf's flags, UTREEXO_FORMAT.md §2a) is not in the block, so both are returned; the log decides.
pub fn candidates(src: &Source, txid: &[u8; 32], vout: u32, height: Option<u32>) -> Result<Vec<Coin>, String> {
    let blocks: Box<dyn Iterator<Item = Result<Block, String>>> = match (src, height) {
        (Source::Dir(d), Some(h)) => Box::new(std::iter::once(json_block(&d.join(format!("block_{h}.json"))))),
        (Source::Dir(d), None) => {
            let mut hs: Vec<(u32, PathBuf)> = std::fs::read_dir(d).map_err(|e| format!("{}: {e}", d.display()))?.flatten()
                .filter_map(|e| { let n = e.file_name().to_string_lossy().to_string();
                                  n.strip_prefix("block_")?.strip_suffix(".json")?.parse().ok().map(|h| (h, e.path())) })
                .collect();
            hs.sort();
            Box::new(hs.into_iter().map(|(_, p)| json_block(&p)))
        }
        (Source::Cli(c), Some(h)) => Box::new(std::iter::once(cli_block(c, h))),
        (Source::Cli(c), None) => {
            // the confirmed transaction's block (needs -txindex, or give --height)
            let mut shown = *txid;
            shown.reverse();
            let j: serde_json::Value = serde_json::from_str(&node(c, &["getrawtransaction", &swiftsync_verify::hex(&shown), "true"])?)
                .map_err(|e| e.to_string())?;
            let bh = j["blockhash"].as_str().ok_or("the node knows no block for that transaction (unconfirmed?)")?.to_string();
            let hj: serde_json::Value = serde_json::from_str(&node(c, &["getblockheader", &bh])?).map_err(|e| e.to_string())?;
            let h = hj["height"].as_u64().ok_or("no height")? as u32;
            Box::new(std::iter::once(cli_block(c, h)))
        }
    };
    // a txid can repeat (BIP30's two pairs): every block holding it is a reading, the log keeps the one it holds
    let mut out = Vec::new();
    for b in blocks {
        let (h, txs, mtp) = b?;
        for (i, t) in txs.iter().enumerate() {
            if libzync::facts::txid(t) != *txid {
                continue;
            }
            let (value, spk) = t.outputs.get(vout as usize).ok_or("that transaction has no such output")?;
            out.extend([false, true].iter().map(|&tracked| Coin {
                txid: *txid, vout, value: *value, script_pubkey: spk.clone(), height: h, coinbase: i == 0, tracked, creation_mtp: mtp,
            }));
        }
    }
    if out.is_empty() { Err("no block in the source holds that transaction".into()) } else { Ok(out) }
}
