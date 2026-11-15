//! `zync-deposit`: builds a deposit proof bundle (docs/libzync/DEPOSIT_ZAPP.md) for `swiftsync-verify deposit`.
//! It holds no secrets and proves nothing: every part of the bundle is checked by the verifier, and a wrong part
//! fails there. Before writing, it runs that verifier on what it built and refuses a bundle that does not verify.
//!
//!   zync-deposit --core <core.rcpt> --history <history.bin> [--headers <rcpt>]... [--headers-file <file>]
//!                --height <h> --txid <txid> (--block <raw block hex file> | --bitcoin-cli "<cmd>")
//!                [--tip-path <file>] [--network mainnet|signet] -o <deposit.json>
//!   zync-deposit mmr-path <headers file> <tip height> <height>     an MMR path, one 64-hex hash per line
//!
//! `--history`: the core proving run's history.bin (libzync::history), for the block's leaf in the core proof's block
//! forest (its coin state and undo hash). `--headers-file`: "height hex80" lines from genesis (as the header zAPP is
//! proved from), for the MMR paths of the core tip and of a block above it; `--tip-path`: the core tip's path, made
//! before with `mmr-path` (then the headers file is needed only for a block above the core tip). `--block`: the raw block as
//! `getblock <hash> 0` prints it; `--bitcoin-cli` asks a node for it instead.
use swiftsync_verify::deposit::{core_state, make_bundle, verify_deposit, DepositAsk};
use swiftsync_verify::headers::{HeaderState, HEADERS_IMAGE_ID};
use swiftsync_verify::{hex, hex32, Image, Verdict};

fn die(m: &str) -> ! {
    eprintln!("zync-deposit: {m}");
    std::process::exit(1);
}

fn unhex(s: &str) -> Vec<u8> {
    swiftsync_verify::unhex(s.trim()).unwrap_or_else(|| die("not hex"))
}

fn node(cmd: &str, args: &[&str]) -> String {
    let mut p = cmd.split_whitespace();
    let prog = p.next().unwrap_or_else(|| die("empty --bitcoin-cli"));
    let o = std::process::Command::new(prog).args(p).args(args).output().unwrap_or_else(|e| die(&format!("{prog}: {e}")));
    if !o.status.success() { die(&format!("{cmd} {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())); }
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Block hashes 0..=tip from a "height hex80" file that starts at genesis and links.
fn hashes_from(file: &str, tip: i64) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = Vec::new();
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| die(&format!("{file}: {e}")));
    for l in text.lines().filter(|l| !l.trim().is_empty()) {
        let mut f = l.split_whitespace();
        let h: i64 = f.next().and_then(|x| x.parse().ok()).unwrap_or_else(|| die("headers file: a line without a height"));
        if h > tip { break; }
        if h != out.len() as i64 { die(&format!("headers file: not consecutive from genesis at {h}")); }
        let b = unhex(f.next().unwrap_or_else(|| die("headers file: a line without a header")));
        if b.len() != 80 { die(&format!("headers file: header {h} is not 80 bytes")); }
        if let Some(p) = out.last() { if b[4..36] != p[..] { die(&format!("headers file: {h} does not link to {}", h - 1)); } }
        out.push(libzync::facts::header_hash(&b.try_into().unwrap()));
    }
    if out.len() as i64 != tip + 1 { die(&format!("headers file ends below the header tip {tip}")); }
    out
}

fn path(hs: &[[u8; 32]], h: u64) -> Vec<[u8; 32]> {
    swiftsync_verify::mmr::path(hs, h).unwrap_or_else(|| die(&format!("block {h} is not in 0..={}", hs.len() - 1)))
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.first().map(|s| s.as_str()) == Some("mmr-path") {
        let n = |i: usize| -> u64 { a.get(i).and_then(|x| x.parse().ok()).unwrap_or_else(|| die("mmr-path <headers file> <tip> <height>")) };
        let hs = hashes_from(a.get(1).unwrap_or_else(|| die("mmr-path <headers file> <tip> <height>")), n(2) as i64);
        for p in path(&hs, n(3)) { println!("{}", hex(&p)); }
        return;
    }
    let get = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let all = |k: &str| a.windows(2).filter(|w| w[0] == k).map(|w| w[1].clone()).collect::<Vec<_>>();
    let need = |k: &str| get(k).unwrap_or_else(|| die(&format!("{k} is required (see the source header)")));
    let network = get("--network").unwrap_or("mainnet".into());
    let core = std::fs::read(need("--core")).unwrap_or_else(|e| die(&format!("--core: {e}")));
    let cs = core_state(&core).unwrap_or_else(|e| die(&e));
    let height: u32 = need("--height").parse().unwrap_or_else(|_| die("--height: not a height"));
    let mut txid = hex32(&need("--txid").to_lowercase()).unwrap_or_else(|| die("--txid: 64 hex digits"));
    txid.reverse();
    let headers: Vec<Vec<u8>> = all("--headers").iter().map(|f| std::fs::read(f).unwrap_or_else(|e| die(&format!("{f}: {e}")))).collect();
    let raw_block = match (get("--block"), get("--bitcoin-cli")) {
        (Some(f), _) => unhex(&std::fs::read_to_string(&f).unwrap_or_else(|e| die(&format!("{f}: {e}")))),
        (None, Some(cli)) => {
            let h = node(&cli, &["getblockhash", &height.to_string()]);
            unhex(&node(&cli, &["getblock", &h, "0"]))
        }
        _ => die("give --block <raw block hex file> or --bitcoin-cli \"<cmd>\""),
    };
    // the header chain's tip, from its last receipt's journal (the verifier checks the receipts)
    let htip: Option<HeaderState> = headers.last().map(|b| {
        let r: risc0_zkvm::Receipt = bincode::deserialize(b).unwrap_or_else(|e| die(&format!("header receipt: {e}")));
        r.journal.decode::<([u8; 32], HeaderState)>().unwrap_or_else(|e| die(&format!("header journal: {e}"))).1
    });
    // --tip-path <file>: the core tip's MMR path, already made (`mmr-path`), instead of reading the headers file
    let read_path = |f: &str| -> Vec<[u8; 32]> {
        std::fs::read_to_string(f).unwrap_or_else(|e| die(&format!("{f}: {e}"))).lines().map(str::trim).filter(|l| !l.is_empty())
            .map(|l| hex32(&l.to_lowercase()).unwrap_or_else(|| die(&format!("{f}: not a 64-hex-digit hash per line")))).collect()
    };
    let given_tip_path = get("--tip-path").map(|f| read_path(&f));
    let hs = match (&htip, &given_tip_path) {
        (Some(s), None) => Some(hashes_from(&get("--headers-file").unwrap_or_else(|| die("header receipts need --headers-file or --tip-path")), s.height)),
        (Some(_), Some(_)) if height > cs.height => Some(hashes_from(&need("--headers-file"), htip.as_ref().unwrap().height)),
        _ => None,
    };
    let tip_path = match (&given_tip_path, &hs) { (Some(p), _) => p.clone(), (None, Some(h)) => path(h, cs.height as u64), _ => Vec::new() };
    let block_path = match &hs { Some(h) if height > cs.height => Some(path(h, height as u64)), _ => None };
    let history = get("--history").map(|f| std::fs::read(&f).unwrap_or_else(|e| die(&format!("{f}: {e}"))));
    let b = make_bundle(&network, &core, &headers, history.as_deref(), &raw_block, height, &txid, &tip_path, block_path.as_deref())
        .unwrap_or_else(|e| die(&e));
    // self-check with the verifier: the transaction at depth >= 1, a block above the core tip allowed (it is flagged)
    let img = Image::by_network(&network).unwrap_or_else(|| die("unknown network"));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let ask = DepositAsk { txid, min_depth: 1, allow_pow_only_block: true, ..Default::default() };
    let rep = verify_deposit(&b, &ask, &network, img, HEADERS_IMAGE_ID, now);
    if rep.verdict == Verdict::Fail {
        for (_, w) in rep.checks.iter().filter(|c| !c.0) { eprintln!("  [FAIL] {w}"); }
        die("the bundle does not verify: not written");
    }
    let out = need("-o");
    let j = serde_json::to_string(&b).unwrap();
    std::fs::write(&out, &j).unwrap_or_else(|e| die(&format!("{out}: {e}")));
    let s = rep.statement.unwrap();
    eprintln!("zync-deposit: wrote {out} ({} bytes): tx at position {} of block {height}, {} confirmation(s) as of tip {} \
               ({} fully validated through {}){}", j.len(), b.block.index, s.confirmations, s.tip_height, s.validated_confirmations,
              s.validated_through, if s.block_fully_validated { "" } else { "; the block is ABOVE the core tip: proof of work only" });
}
