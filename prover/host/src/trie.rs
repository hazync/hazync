//! libzync: the counting trie, host side (E2 step 3). The trie STORE (`libzync::trie_store`) kept beside the coin log:
//! one block at a time it serves the trie receipt's witness (guest mode 17), and after the block's aggregate is proved
//! it applies the same update list and checks the new root is the one the chain state committed. A `state_<h>.trie`
//! snapshot sits next to each `state_<h>.json` / `state_<h>.coinlog`, so a run can resume or be anchored on it.
//!
//! The update list comes from the aggregate's collect run ("trie: R <key> <spender>" / "trie: I <key> <spent>
//! <spender> <value>"); the same parse and the same mode-17 input serve the measurement path (`lz::cl_synth_around`,
//! with a synthesised old trie) and the prover (`lz::lz_prove_range_cmd`, with this store).

use libzync::coinlog::W;
use libzync::trie_store::TrieStore;
use libzync::ttree::Key;
use risc0_zkvm::ExecutorEnv;
use crate::ChainState;
use crate::lz::hx_bytes;

pub(crate) type Marks = Vec<(Key, W)>;
pub(crate) type Inserts = Vec<(Key, Option<W>, u64)>;

/// The trie store for a prove-range run (None in images without the trie).
pub(crate) static TS: std::sync::Mutex<Option<TrieStore>> = std::sync::Mutex::new(None);

/// The collect run's full update list, in the order the aggregate's digest covers it.
pub(crate) fn trie_parse(text: &str) -> (Marks, Inserts) {
    let hw = |x: &str| -> W {
        assert_eq!(x.len(), 64, "trie: spender hex");
        core::array::from_fn(|i| u32::from_str_radix(&x[8 * i..8 * i + 8], 16).expect("trie: spender hex"))
    };
    let (mut marks, mut ins) = (Vec::new(), Vec::new());
    for l in text.lines() {
        let Some(i) = l.find("trie: ") else { continue };
        let f: Vec<&str> = l[i + 6..].split_whitespace().collect();
        match f.as_slice() {
            ["R", k, sp] => marks.push((hx_bytes(k).try_into().expect("trie key: 64 bytes"), hw(sp))),
            ["I", k, fl, sp, v] => ins.push((hx_bytes(k).try_into().expect("trie key: 64 bytes"),
                                             (*fl == "1").then(|| hw(sp)), v.parse().expect("trie: value"))),
            _ => {}
        }
    }
    (marks, ins)
}

/// Guest mode 17's input: [witness words, marks, inserts, perm length], the witness, marks ×24, inserts ×27, the
/// update order (the guest checks it is a permutation and that the keys strictly increase in it).
pub(crate) fn trie_env(b: &mut risc0_zkvm::ExecutorEnvBuilder, wit: &[u32], marks: &Marks, ins: &Inserts) {
    let ks: Vec<(W, W)> = marks.iter().map(|x| libzync::ttree::split(&x.0)).chain(ins.iter().map(|x| libzync::ttree::split(&x.0))).collect();
    let mut perm: Vec<u32> = (0..ks.len() as u32).collect();
    perm.sort_by(|&a, &c| ks[a as usize].cmp(&ks[c as usize]));
    let kw = |k: &Key| -> Vec<u32> { k.chunks_exact(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect() };
    let mut mw: Vec<u32> = Vec::with_capacity(marks.len() * 24);
    for (k, sp) in marks { mw.extend(kw(k)); mw.extend_from_slice(sp); }
    let mut iw: Vec<u32> = Vec::with_capacity(ins.len() * 27);
    for (k, sp, v) in ins { iw.extend(kw(k)); iw.push(sp.is_some() as u32); iw.extend_from_slice(&sp.unwrap_or([0; 8])); iw.push(*v as u32); iw.push((*v >> 32) as u32); }
    b.segment_limit_po2(crate::seg_po2());
    b.write(&17u32).unwrap();
    b.write_slice(&[wit.len() as u32, marks.len() as u32, ins.len() as u32, perm.len() as u32]);
    b.write_slice(wit);
    if !mw.is_empty() { b.write_slice(&mw); }
    if !iw.is_empty() { b.write_slice(&iw); }
    if !perm.is_empty() { b.write_slice(&perm); }
}

/// The trie receipt's journal as words: [KIND_TRIE, old root, new root, digest].
pub(crate) fn trie_journal_words(j: &[u8]) -> [u32; libzync::ttree::TRIE_JOURNAL_WORDS] {
    assert_eq!(j.len(), 4 * libzync::ttree::TRIE_JOURNAL_WORDS, "trie journal length");
    core::array::from_fn(|i| u32::from_le_bytes(j[4 * i..4 * i + 4].try_into().unwrap()))
}

/// PROVE one block's trie update against the store's current root (prove-range). Returns the receipt for the aggregate.
pub(crate) fn trie_prove(h: u32, marks: &Marks, ins: &Inserts, prev: &ChainState, opts: &risc0_zkvm::ProverOpts) -> risc0_zkvm::Receipt {
    let g = TS.lock().unwrap();
    let st = g.as_ref().expect("trie store not initialised");
    assert_eq!(libzync::coinlog::h_of(&st.root()), prev.trie_root, "block {h}: the trie store is not at the previous state's trie root");
    let wit = st.witness(marks, ins);
    drop(g);
    let mut b = ExecutorEnv::builder();
    trie_env(&mut b, &wit, marks, ins);
    let r = risc0_zkvm::default_prover().prove_with_opts(b.build().unwrap(), methods::METHOD_ELF, opts)
        .unwrap_or_else(|e| panic!("block {h}: trie update proof (mode 17) failed: {e}")).receipt;
    r.verify(methods::METHOD_ID).expect("trie receipt verifies");
    let j = trie_journal_words(&r.journal.bytes);
    assert!(j[0] == libzync::ttree::KIND_TRIE && j[1..9] == libzync::coinlog::w_of(&prev.trie_root)[..],
            "block {h}: the trie receipt is not from the previous trie root");
    println!("  block {h}: trie update proved: {} marks, {} inserts, witness {} words", marks.len(), ins.len(), wit.len());
    r
}

/// After the block's aggregate is proved: apply its update list to the store; the new root must be the committed one.
pub(crate) fn trie_commit(marks: &Marks, ins: &Inserts, st: &ChainState) {
    let mut g = TS.lock().unwrap();
    let ts = g.as_mut().expect("trie store not initialised");
    let new = ts.apply(marks, ins).unwrap_or_else(|e| panic!("block {}: the trie store refused the proved update list: {e:?}", st.height));
    assert_eq!(libzync::coinlog::h_of(&new), st.trie_root, "block {}: the trie store's new root != the chain state's", st.height);
    ts.height = st.height as u64;
    ts.hash = st.tip_hash;
}

/// `state_<h>.json` -> `state_<h>.trie`; `anchor.json` -> `anchor.trie`.
pub(crate) fn trie_path_for(state_json: &str) -> String {
    match state_json.strip_suffix(".json") { Some(s) => format!("{s}.trie"), None => format!("{state_json}.trie") }
}

/// Atomic, as the coin log's: a run killed mid-write must not leave a short file a resume would then refuse.
pub(crate) fn trie_save(path: &str) {
    let b = TS.lock().unwrap().as_ref().expect("trie store not initialised").to_bytes();
    let tmp = format!("{path}.tmp");
    let f = std::fs::File::create(&tmp).and_then(|mut f| { use std::io::Write as _; f.write_all(&b)?; f.sync_data() });
    f.unwrap_or_else(|e| panic!("{tmp}: {e}"));
    std::fs::rename(&tmp, path).unwrap_or_else(|e| panic!("{path}: {e}"));
    if let Some(dir) = std::path::Path::new(path).parent() {
        crate::lz::cl_sync_dir(if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir });
    }
}

/// Load a snapshot and check it is the trie of `st` (its height, block and root).
pub(crate) fn trie_load(path: &str, st: &ChainState) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e} (a trie image's anchor needs its .trie file)"));
    let ts = TrieStore::from_bytes(&b).unwrap_or_else(|e| panic!("{path}: not a valid trie snapshot ({e:?})"));
    assert!(ts.counts(), "{path}: not a counts-mode trie");
    assert_eq!(libzync::coinlog::h_of(&ts.root()), st.trie_root, "{path}: its root is not the state's trie root");
    assert!(ts.height == st.height as u64 && ts.hash == st.tip_hash, "{path}: a snapshot at {} not {}", ts.height, st.height);
    *TS.lock().unwrap() = Some(ts);
}

/// The trie at the run's start: empty from genesis, the handover receipt's directory's snapshot, or the anchor's.
pub(crate) fn trie_init_at_start(handover: bool, anchor_path: &str, prev: &ChainState) {
    if anchor_path == "genesis" && !handover {
        assert_eq!(prev.trie_root, [0; 32], "genesis: the trie root must be the empty trie's");
        let mut ts = TrieStore::new(true);
        ts.hash = prev.tip_hash;
        *TS.lock().unwrap() = Some(ts);
    } else if handover {
        let p = std::env::var("HAZYNC_HANDOVER_RECEIPT").unwrap();
        let d = std::path::Path::new(&p).parent().map(|x| x.to_string_lossy().to_string())
            .filter(|x| !x.is_empty()).unwrap_or_else(|| ".".to_string());
        trie_load(&format!("{d}/state_{}.trie", prev.height), prev);
    } else {
        trie_load(&trie_path_for(anchor_path), prev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_both_line_kinds_and_ignores_the_rest() {
        let k = "ab".repeat(64);
        let sp = format!("{:08x}{}", 7, "0".repeat(56));
        let log = format!("R0VM[1] trie: R {k} {sp}\nnoise\ntrie: I {k} 1 {sp} 5000\ntrie: I {k} 0 {} 12\n", "0".repeat(64));
        let (m, i) = trie_parse(&log);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].1[0], 7);
        assert_eq!(i.len(), 2);
        assert_eq!((i[0].1.unwrap()[0], i[0].2), (7, 5000));
        assert_eq!((i[1].1, i[1].2), (None, 12));
        assert_eq!(m[0].0, [0xab; 64]);
    }
}
