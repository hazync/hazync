//! The Utreexo roots zyncAPP's host side (docs/libzync/UTREEXO_ZAPP.md §5): a native replay of blocks 1..=T from the
//! block files (`fetch_block_rpc.py`'s JSON), which yields the guest's witness (the same frames as the hints zyncAPP's,
//! HINTS_ZAPP.md §10), then the same derivation the guest runs (`libzync::utreexo_roots::Deriver`), checked against the
//! core state before any proving. Nothing here is trusted by the proof: the guest recomputes every leaf and every forest.
//! The replay is the hints host's (feat/hints-zapp `hints_host.rs`), copied so this branch stands on main alone.
use libzync::coinlog::{self, H};
use libzync::coinlog_forest::Store;
use libzync::facts;
use libzync::utreexo_roots as ur;
use std::collections::HashMap;

pub const MAINNET_GENESIS: &str = "0100000000000000000000000000000000000000000000000000000000000000000000003ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a29ab5f49ffff001d1dac2b7c";
pub const SIGNET_GENESIS: &str = "0100000000000000000000000000000000000000000000000000000000000000000000003ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a008f4d5fae77031e8ad22203";

pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

pub fn genesis(signet: bool) -> [u8; 80] {
    unhex(if signet { SIGNET_GENESIS } else { MAINNET_GENESIS }).try_into().unwrap()
}

/// One block as the guest reads it: header ‖ CompactSize count ‖ every transaction without witness.
pub struct Block {
    pub height: u32,
    pub hash: H,
    pub header: [u8; 80],
    pub stripped: Vec<u8>,
    pub txs: Vec<facts::TxView>,
    pub txids: Vec<H>,
    /// per transaction after the coinbase, its inputs' prevouts in input order (the block file's `prevouts`, from the
    /// node's undo data): what the tip mode's forest source (utreexo_witness.rs) rebuilds each spent leaf from. Empty
    /// when the file has none.
    pub prevouts: Vec<Vec<crate::utreexo_witness::PrevCoin>>,
}

fn rev_hex32(s: &str) -> [u8; 32] {
    let mut b: [u8; 32] = unhex(s).try_into().expect("32-byte hash");
    b.reverse();
    b
}

/// Reads `block_<h>.json` (prover/fetch_block_rpc.py): header fields, the coinbase and every transaction (with witness).
pub fn read_block(dir: &str, h: u32) -> Block {
    let p = format!("{dir}/block_{h}.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap_or_else(|e| panic!("{p}: {e}"));
    assert_eq!(v["height"].as_u64(), Some(h as u64), "{p}: wrong height");
    let mut header = [0u8; 80];
    header[0..4].copy_from_slice(&(v["version"].as_i64().unwrap() as i32).to_le_bytes());
    header[4..36].copy_from_slice(&rev_hex32(v["prev"].as_str().unwrap()));
    header[36..68].copy_from_slice(&rev_hex32(v["merkle"].as_str().unwrap()));
    header[68..72].copy_from_slice(&(v["time"].as_u64().unwrap() as u32).to_le_bytes());
    header[72..76].copy_from_slice(&(v["bits"].as_u64().unwrap() as u32).to_le_bytes());
    header[76..80].copy_from_slice(&(v["nonce"].as_u64().unwrap() as u32).to_le_bytes());
    let mut raws = vec![unhex(v["coinbase_hex"].as_str().unwrap())];
    let mut prevouts = Vec::new();
    for t in v["txs"].as_array().unwrap() {
        raws.push(unhex(t["raw"].as_str().unwrap()));
        if let Some(ps) = t["prevouts"].as_array() {
            prevouts.push(ps.iter().map(|x| crate::utreexo_witness::PrevCoin {
                height: x["coin_height"].as_u64().unwrap_or_else(|| panic!("{p}: a prevout without coin_height")) as u32,
                coinbase: x["coin_is_coinbase"].as_u64().unwrap_or_else(|| panic!("{p}: a prevout without coin_is_coinbase")) == 1,
                value: x["value"].as_u64().unwrap_or_else(|| panic!("{p}: a prevout without value")),
                spk: unhex(x["spk"].as_str().unwrap_or_else(|| panic!("{p}: a prevout without spk"))),
            }).collect());
        }
    }
    if !prevouts.is_empty() && prevouts.len() + 1 != raws.len() {
        panic!("{p}: prevouts for some transactions only");
    }
    let mut txs = Vec::new();
    for r in &raws {
        let mut o = 0;
        let t = facts::parse_tx(r, &mut o).unwrap_or_else(|| panic!("{p}: a transaction does not parse"));
        assert_eq!(o, r.len(), "{p}: trailing bytes in a transaction");
        txs.push(t);
    }
    let mut stripped = header.to_vec();
    put_varint(&mut stripped, txs.len() as u64);
    for t in &txs {
        stripped.extend_from_slice(&t.stripped);
    }
    let txids = txs.iter().map(facts::txid).collect();
    Block { height: h, hash: facts::header_hash(&header), header, stripped, txs, txids, prevouts }
}

fn put_varint(v: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => v.push(n as u8),
        0xfd..=0xffff => { v.push(0xfd); v.extend_from_slice(&(n as u16).to_le_bytes()); }
        0x1_0000..=0xffff_ffff => { v.push(0xfe); v.extend_from_slice(&(n as u32).to_le_bytes()); }
        _ => { v.push(0xff); v.extend_from_slice(&n.to_le_bytes()); }
    }
}

/// UTREEXO_FORMAT.md §2a: the height a coinbase's scriptSig indicates (Core's `CScript() << Y` as its first bytes), or 0.
fn indicated_height(script_sig: &[u8]) -> u64 {
    let Some(&f) = script_sig.first() else { return 0 };
    if (0x51..=0x60).contains(&f) {
        return (f - 0x50) as u64;
    }
    if !(1..=5).contains(&f) || script_sig.len() < 1 + f as usize {
        return 0;
    }
    let b = &script_sig[1..1 + f as usize];
    if b[b.len() - 1] & 0x80 != 0 {
        return 0; // negative
    }
    let mut y = 0u64;
    for (i, x) in b.iter().enumerate() {
        y |= (*x as u64) << (8 * i);
    }
    // must be exactly CScript() << Y: minimal CScriptNum, and Y > 16 (smaller ones are OP_1..OP_16)
    let mut enc = Vec::new();
    let mut t = y;
    while t > 0 {
        enc.push((t & 0xff) as u8);
        t >>= 8;
    }
    if enc.last().is_some_and(|x| x & 0x80 != 0) {
        enc.push(0);
    }
    if y <= 16 || enc != b {
        return 0;
    }
    y
}

fn coinbase_script_sig(stripped: &[u8]) -> &[u8] {
    // version 4 ‖ count 1 (a coinbase has one input) ‖ prevout 36 ‖ varint length ‖ scriptSig
    let mut o = 41;
    let l = match stripped[o] {
        0xfd => { o += 3; u16::from_le_bytes([stripped[42], stripped[43]]) as usize }
        x => { o += 1; x as usize }
    };
    &stripped[o..o + l]
}

struct Coin {
    pos: Option<u64>,
    ord: usize,
    value: u64,
    spk: Vec<u8>,
    height: u32,
    cb: bool,
    tracked: bool,
    leaf: H,
}

/// What the replay keeps per block for the witness: the positions of the transactions with flag bit 1, the coin-log
/// positions it appended (in order), the coin state after it and its undo hash.
pub struct BlockFacts {
    pub tracked: Vec<u32>,
    pub appended: std::ops::Range<u64>,
    pub state: H,
    pub undo: H,
    pub leaf: H,
}

pub struct Replay {
    pub blocks: Vec<BlockFacts>, // index h - 1
    pub spends: Vec<Option<(u32, u32, u32)>>,
    pub utxo_value: u64,
    pub store: Store,
    pub block_roots: Vec<Option<H>>,
    pub block_leaves: u64,
}

/// Replays blocks 1..=t: the coin log (with every flip and its spender), the coin value, each block's undo hash, the
/// tracked bits and the block forest. The block bytes come from `get` (cached by the caller if it wants them again).
pub fn replay(t: u32, signet: bool, mut get: impl FnMut(u32) -> Block) -> Replay {
    let bip34: u32 = if signet { 1 } else { 227_931 };
    let bip30: &[(u32, H)] = if signet { &[] } else { &ur::MAINNET_BIP30 };
    let g = genesis(signet);
    let mut times = vec![u32::from_le_bytes(g[68..72].try_into().unwrap())];
    let mut blk: Vec<Option<H>> = Vec::new();
    let mut blk_n = 0u64;
    let push_blk = |roots: &mut Vec<Option<H>>, n: &mut u64, leaf: H| {
        let mut node = leaf;
        let mut k = 0;
        loop {
            if k == roots.len() { roots.push(None); }
            match roots[k].take() {
                Some(r) => { node = facts::block_parent(0, &r, &node); k += 1; }
                None => { roots[k] = Some(node); break; }
            }
        }
        *n += 1;
    };
    push_blk(&mut blk, &mut blk_n, facts::block_leaf(&facts::header_hash(&g), 0, &facts::utxo_state(0, 0, &[])));
    let mut live: HashMap<(H, u32), Coin> = HashMap::new();
    let mut repeatable: HashMap<H, u32> = HashMap::new(); // tracked coinbase txid -> its live coins
    let mut store = Store::default();
    let mut spends: Vec<Option<(u32, u32, u32)>> = Vec::new();
    let mut value: u64 = 0;
    let mut blocks = Vec::with_capacity(t as usize);
    for h in 1..=t {
        let b = get(h);
        let mtp = ur::median_time(&times);
        let overwritten = bip30.iter().any(|(bh, bx)| *bh == h && *bx == b.hash);
        let mut undo = libzync::undo::UndoHasher::new();
        let mut tracked = Vec::new();
        let mut new: Vec<(H, u32)> = Vec::new();
        let mut gone = Vec::new(); // block-local ordinals spent in the block
        for (ti, tx) in b.txs.iter().enumerate() {
            let txid = b.txids[ti];
            let is_tracked = if ti == 0 {
                let y = indicated_height(coinbase_script_sig(&tx.stripped));
                // the two grandfathered overwrites (91,842 / 91,880) are never a repeat (prep's tracked.rs `coinbase_flags`)
                let grandfathered = !signet && (h == 91_842 || h == 91_880);
                (h < bip34 && (h as u64) < y && y < 100_000_000) || (!grandfathered && repeatable.get(&txid) == Some(&0))
            } else {
                let mut all = true;
                for (vin, (pt, pv)) in tx.inputs.iter().enumerate() {
                    let c = live.remove(&(*pt, *pv)).unwrap_or_else(|| panic!("block {h} tx {ti}: spends a coin that is not live"));
                    undo.coin(c.height, c.cb, c.value, &c.spk);
                    all &= c.tracked;
                    match c.pos {
                        Some(p) => {
                            store.set(p, coinlog::spent(&c.leaf, &coinlog::spender(h, ti as u32, vin as u32)));
                            spends[p as usize] = Some((h, ti as u32, vin as u32));
                            value -= c.value;
                        }
                        None => gone.push(c.ord),
                    }
                    if c.tracked && c.cb {
                        if let Some(n) = repeatable.get_mut(pt) { *n -= 1; }
                    }
                }
                all
            };
            if is_tracked {
                tracked.push(ti as u32);
            }
            if ti == 0 && overwritten {
                continue;
            }
            let mut counted = 0;
            for (v, (val, spk)) in tx.outputs.iter().enumerate() {
                if ur::unspendable_script(spk) {
                    continue;
                }
                counted += 1;
                let leaf = facts::coin_leaf(&txid, v as u32, *val, spk, h, ti == 0, is_tracked, mtp);
                let ord = new.len();
                new.push((txid, v as u32));
                live.insert((txid, v as u32), Coin { pos: None, ord, value: *val, spk: spk.clone(), height: h, cb: ti == 0, tracked: is_tracked, leaf });
            }
            // as prep (prep.rs `tracked_new`, `.filter(|t| t.1 > 0)`): a tracked coinbase with no spendable output never
            // enters the BIP30 tree, so it is never REPEATABLE and a repeat of it is not tracked
            if ti == 0 && is_tracked && counted > 0 {
                repeatable.insert(txid, counted);
            }
        }
        gone.sort_unstable();
        let first = store.len();
        for (ord, k) in new.iter().enumerate() {
            if gone.binary_search(&ord).is_ok() {
                continue;
            }
            let c = live.get_mut(k).unwrap();
            c.pos = Some(store.len());
            store.push(c.leaf);
            spends.push(None);
            value += c.value;
        }
        let undo_h = undo.finish();
        let state = facts::utxo_state(store.len(), value, &store.roots());
        let leaf = ur::block_leaf_undo(&b.hash, h, &state, &undo_h);
        push_blk(&mut blk, &mut blk_n, leaf);
        times.push(u32::from_le_bytes(b.header[68..72].try_into().unwrap()));
        if times.len() > 11 { times.remove(0); }
        blocks.push(BlockFacts { tracked, appended: first..store.len(), state, undo: undo_h, leaf });
    }
    Replay { blocks, spends, utxo_value: value, store, block_roots: ur::trimmed(&blk), block_leaves: blk_n }
}

/// One block's frame for the guest (HINTS_ZAPP.md §10's layout): the block without witnesses, its tracked positions,
/// each coin-log append's status at H (0, or the spender's height, tx, vin), the coin state after it and its undo hash.
pub struct Frame {
    pub stripped: Vec<u8>,
    pub tracked: Vec<u32>,
    pub status: Vec<u32>,
    pub state: H,
    pub undo: H,
}

pub fn frame(r: &Replay, b: &Block, stop: u32) -> Frame {
    let f = &r.blocks[b.height as usize - 1];
    let mut status = Vec::new();
    for p in f.appended.clone() {
        match r.spends[p as usize] {
            Some((sh, tx, vin)) if sh <= stop => status.extend_from_slice(&[sh, tx, vin]),
            _ => status.push(0),
        }
    }
    Frame { stripped: b.stripped.clone(), tracked: f.tracked.clone(), status, state: f.state, undo: f.undo }
}

pub fn restrip(b: &Block, txs: &[&[u8]]) -> Vec<u8> {
    let mut s = b.header.to_vec();
    put_varint(&mut s, txs.len() as u64);
    for t in txs {
        s.extend_from_slice(t);
    }
    s
}

/// A NEGATIVE TEST's change to the frames (UTREEXO_TAMPER, execute only): each must make the guest refuse.
pub fn tamper(case: &str, fr: &mut [Frame], blocks: &[Block], tail: &mut [H]) -> Result<String, String> {
    let first_spent = |fr: &[Frame]| -> Option<(usize, usize)> {
        for (i, f) in fr.iter().enumerate() {
            let mut k = 0;
            while k < f.status.len() {
                if f.status[k] != 0 { return Some((i, k)); }
                k += 1;
            }
        }
        None
    };
    let first_unspent = |fr: &[Frame]| -> Option<(usize, usize)> {
        for (i, f) in fr.iter().enumerate() {
            let mut k = 0;
            while k < f.status.len() {
                if f.status[k] == 0 { return Some((i, k)); }
                k += 3;
            }
        }
        None
    };
    let multi = blocks.iter().position(|b| b.txs.len() >= 2);
    Ok(match case {
        "spent_as_unspent" => { let (i, k) = first_spent(fr).ok_or("no spent coin below H")?; fr[i].status.splice(k..k + 3, [0]); format!("block {}: a spent coin claimed unspent (a Utreexo leaf where Floresta has Empty)", i + 1) }
        "unspent_as_spent" => { let (i, k) = first_unspent(fr).ok_or("no unspent coin")?; let n = fr.len() as u32; fr[i].status.splice(k..k + 1, [n, 1, 0]); format!("block {}: an unspent coin claimed spent at H (Empty where Floresta has a leaf)", i + 1) }
        "wrong_spender" => { let (i, k) = first_spent(fr).ok_or("no spent coin below H")?; fr[i].status[k + 2] += 1; format!("block {}: a spender's input index + 1", i + 1) }
        "tracked" => { fr[0].tracked = vec![0]; "block 1: its coinbase claimed tracked".into() }
        "state" => { fr[0].state[0] ^= 1; "block 1: its coin state flipped".into() }
        "undo" => { fr[0].undo[0] ^= 1; "block 1: its undo hash flipped".into() }
        "header" => { fr[0].stripped[76] ^= 1; "block 1: its nonce flipped".into() }
        "drop_tx" => { let i = multi.ok_or("no block with two transactions")?; let b = &blocks[i];
                       let t: Vec<&[u8]> = b.txs[..b.txs.len() - 1].iter().map(|t| t.stripped.as_slice()).collect(); fr[i].stripped = restrip(b, &t); format!("block {}: its last transaction dropped", i + 1) }
        "swap_tx" => { let i = multi.ok_or("no block with two transactions")?; let b = &blocks[i];
                       let mut t: Vec<&[u8]> = b.txs.iter().map(|t| t.stripped.as_slice()).collect(); t.swap(0, 1); fr[i].stripped = restrip(b, &t); format!("block {}: transactions 0 and 1 swapped", i + 1) }
        "tail" => { let t = tail.first_mut().ok_or("no leaf above H (H = T)")?; t[0] ^= 1; "the first block-forest leaf above H flipped".into() }
        x => return Err(format!("UTREEXO_TAMPER: unknown case {x}")),
    })
}

/// The frames of blocks `lo..=hi` as guest words.
pub fn frame_words(fr: &[Frame]) -> Vec<u32> {
    let mut w = Vec::new();
    for f in fr {
        w.extend_from_slice(&[f.stripped.len() as u32, f.tracked.len() as u32, f.status.len() as u32]);
        w.extend(f.stripped.chunks(4).map(|c| { let mut x = [0u8; 4]; x[..c.len()].copy_from_slice(c); u32::from_le_bytes(x) }));
        w.extend_from_slice(&f.tracked);
        w.extend_from_slice(&f.status);
        w.extend_from_slice(&coinlog::w_of(&f.state));
        w.extend_from_slice(&coinlog::w_of(&f.undo));
    }
    w
}
