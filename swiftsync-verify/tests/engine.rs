//! `verify_engine` on a real engine proof (engine/VERIFIER.md step 4): the engine chain 1..200 from genesis, its proof
//! of block 200 (keys-ch: node format 2 + C heap; `hzengine wasm-inputs` wrote proof.hze, vk.bin and the key's pin),
//! checked by the statement checks; then the negatives, and the malformed files the receipt tests covered.
use swiftsync_verify::*;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine200");

fn rd(n: &str) -> Vec<u8> {
    std::fs::read(format!("{DIR}/{n}")).unwrap_or_else(|e| panic!("{DIR}/{n}: {e}"))
}

/// The engine image as a record: the mainnet record (no image until the Stage 4 freeze) with this engine image's id.
fn image() -> &'static Image {
    let m = Image::by_network("mainnet").unwrap();
    let id = String::from_utf8(rd("image_id")).unwrap().trim().to_string();
    Box::leak(Box::new(Image {
        network: m.network,
        method_id_hex: Box::leak(id.into_boxed_str()),
        profile: m.profile,
        genesis_hash_hex: m.genesis_hash_hex,
        genesis_time: m.genesis_time,
        genesis_bits: m.genesis_bits,
        genesis_work: m.genesis_work,
        min_work_hex: m.min_work_hex,
        genesis_anchor_hex: m.genesis_anchor_hex,
        checkpoint_height: m.checkpoint_height,
        lineage: &[],
        engine_key: None,
    }))
}

fn pin() -> [u8; 32] {
    hex32(String::from_utf8(rd("vk.sha256")).unwrap().trim()).unwrap()
}

fn run(file: &[u8], img: &'static Image, vk: &[u8], pin: [u8; 32]) -> Report {
    verify_engine(&Inputs { receipt: file, image: img, anchor: None, now: 1_800_000_000 }, &EngineKey { bytes: vk, pin })
}

fn failed(r: &Report) -> Vec<String> {
    r.checks.iter().filter(|(ok, _)| !ok).map(|(_, s)| s.clone()).collect()
}

#[test]
fn engine_chain_block_200() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let r = run(&file, img, &vk, pin());
    for (ok, s) in &r.checks {
        eprintln!("  [{}] {s}", if *ok { "ok" } else { "FAIL" });
    }
    // every check passes; the verdict is Demo only because 200 blocks are far below the minimum chain work
    assert!(failed(&r).is_empty(), "failed: {:?}", failed(&r));
    assert_eq!(r.verdict, Verdict::Demo);
    assert!(r.low_work);
    let cs = r.state.as_ref().unwrap();
    assert_eq!((cs.height, cs.profile), (200, 0xf1));
    assert!(r.checks[0].1.starts_with("engine proof verifies against image"));
}

#[test]
fn engine_negatives() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let fails = |r: &Report, want: &str| {
        assert_eq!(r.verdict, Verdict::Fail);
        let f = failed(r);
        assert!(f.iter().any(|s| s.contains(want)), "want a failed check containing {want:?}, got {f:?}");
        eprintln!("  refused: {}", f[0]);
    };
    // the journal (the file's tail) with one bit changed: the public values no longer match
    let mut j = file.clone();
    let n = j.len() - 40;
    j[n] ^= 1;
    fails(&run(&j, img, &vk, pin()), "public values are not SHA-256 of this journal");
    // a proof byte changed
    let mut p = file.clone();
    p[8 + 1000] ^= 1;
    let r = run(&p, img, &vk, pin());
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(r.state.is_none(), "no statement from a proof that did not verify");
    // a key that is not the pinned one, and the right key under another pin
    let mut k = vk.clone();
    let n = k.len() - 1;
    k[n] ^= 1;
    fails(&run(&file, img, &k, pin()), "not the image's pinned key");
    let mut pin2 = pin();
    pin2[0] ^= 1;
    fails(&run(&file, img, &vk, pin2), "not the image's pinned key");
    // another image id
    fails(&run(&file, Image::by_network("mainnet").unwrap().with_method_id(&"ab".repeat(32)).unwrap(), &vk, pin()),
          "not the published image's");
    // not an engine proof file: a RISC Zero receipt's first bytes, and a truncated file
    fails(&run(&[1, 0, 0, 0, 7, 7, 7, 7, 7], img, &vk, pin()), "the file is an engine proof");
    fails(&run(&file[..6], img, &vk, pin()), "the file is an engine proof");
    // plain verify on the network record (no image) and on a RISC Zero receipt's bytes: refused, with the reason
    assert!(is_engine_proof(&file));
    let r = verify(&Inputs { receipt: &file, image: Image::by_network("mainnet").unwrap(), anchor: None, now: 1_800_000_000 });
    fails(&r, "no engine image is published for mainnet yet");
    let r = verify(&Inputs { receipt: &[1, 0, 0, 0, 7, 7, 7, 7], image: img.with_engine_key(vk.clone()), anchor: None, now: 1_800_000_000 });
    fails(&r, "v0.27.1");
}

/// Coin facts (zync-path from the engine chain's history.bin) checked against the verified engine state, as
/// `swiftsync-verify --coin` does after `verify`: block 1's coinbase unspent at 200 (with its preimage, which an UNSPENT
/// answer requires), block 9's coinbase spent in block 170 (Satoshi to Hal).
#[test]
fn engine_coin_facts() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let r = run(&file, img, &vk, pin());
    let cs = r.state.as_ref().unwrap();
    let fact = |n: &str| -> CoinFact { serde_json::from_slice(&rd(n)).unwrap() };
    for n in ["coin_unspent_0.json", "coin_spent_8.json"] {
        let ok = check_coin_fact(cs, r.undo_from, &fact(n));
        eprintln!("  {n}: {ok:?}");
        assert!(ok.is_ok(), "{n}: {ok:?}");
    }
    assert!(check_coin_fact(cs, r.undo_from, &fact("coin_spent_8.json")).unwrap().contains("170"), "spent in block 170");
    // negatives: a path node changed; the preimage's value changed; the unspent answer without its preimage; the spent
    // coin claimed unspent
    let mut f = fact("coin_unspent_0.json");
    let p0 = f.path[0].clone();
    f.path[0] = format!("{}{}", if p0.starts_with('0') { "1" } else { "0" }, &p0[1..]);
    assert!(check_coin_fact(cs, r.undo_from, &f).is_err(), "a changed path node");
    let mut f = fact("coin_unspent_0.json");
    f.preimage.as_mut().unwrap().value += 1;
    assert!(check_coin_fact(cs, r.undo_from, &f).is_err(), "a preimage that is not the coin's");
    let mut f = fact("coin_unspent_0.json");
    f.preimage = None;
    assert!(check_coin_fact(cs, r.undo_from, &f).is_err(), "an unspent answer without its preimage");
    let mut f = fact("coin_spent_8.json");
    f.spent_by = None;
    assert!(check_coin_fact(cs, r.undo_from, &f).is_err(), "a spent coin claimed unspent");
}

/// The key in the image RECORD (what a published engine image will carry): plain `verify` checks an engine file, so
/// every caller of `verify` (corecheck, the zAPPs) gets engine proofs with no new parameter.
#[test]
fn engine_key_in_the_record() {
    let (file, vk) = (rd("proof.hze"), rd("vk.bin"));
    let img = image().with_engine_key(vk.clone());
    assert_eq!(img.engine_key.unwrap().pin_hex, hex(&pin()), "the record's pin is the key file's SHA-256");
    let r = verify(&Inputs { receipt: &file, image: img, anchor: None, now: 1_800_000_000 });
    assert!(failed(&r).is_empty(), "failed: {:?}", failed(&r));
    assert_eq!(r.verdict, Verdict::Demo);
    // a record without a key: refused with the reason
    let r = verify(&Inputs { receipt: &file, image: image(), anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(failed(&r)[0].contains("has no engine verifying key"), "{:?}", failed(&r));
    // a record whose key is another file: the pin is that file's, so the engine check refuses the key at the image
    // id (the pin is "right" for the wrong key: what a record built from a wrong key would do)
    let mut k2 = vk.clone();
    let n = k2.len() - 1;
    k2[n] ^= 1;
    let r = verify(&Inputs { receipt: &file, image: image().with_engine_key(k2), anchor: None, now: 1_800_000_000 });
    assert_eq!(r.verdict, Verdict::Fail);
    eprintln!("  record with another key: {}", failed(&r)[0]);
}

/// proof_journal's framing is verify-core's: the same journal as unpack, and the verified state's own journal.
#[test]
fn proof_journal_reads_the_engine_file() {
    let file = rd("proof.hze");
    let j = proof_journal(&file).unwrap();
    assert_eq!(&j.bytes[..], hazync_engine_verify_unpack_journal(&file));
    let cs: ChainState = decode_journal(&j).unwrap();
    assert_eq!(cs.height, 200);
    assert!(proof_journal(&file[..6]).is_err());
}

fn hazync_engine_verify_unpack_journal(file: &[u8]) -> &[u8] {
    // the layout as verify-core writes it (HZE1, u32 LE proof length, proof, journal)
    let n = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    &file[8 + n..]
}

/// Counting-trie facts at the engine chain's tip (zync-path trie on the engine chain's trie at 200, node format 2), each
/// with its claimed answer, and the coin journey of Hal's coin (zync-path journey): checked against the verified
/// engine state, as `swiftsync-verify --trie / --journey` do after `verify`.
#[test]
fn engine_trie_facts_and_journey() {
    let (file, vk, img) = (rd("proof.hze"), rd("vk.bin"), image());
    let r = run(&file, img, &vk, pin());
    let cs = r.state.as_ref().unwrap();
    let trie = |n: &str| -> TrieFact { serde_json::from_slice(&rd(n)).unwrap() };
    for n in ["trie_coin_hal.json", "trie_balance_hal.json", "trie_history_hal.json", "trie_coin_absent.json"] {
        let v = check_trie_fact(cs, r.undo_from, &trie(n));
        eprintln!("  {n}: {v:?}");
        assert!(v.is_ok(), "{n}: {v:?}");
    }
    // a claimed answer the trie does not give: refused
    for (n, bad) in [("trie_coin_hal.json", "unspent:1000000001"), ("trie_balance_hal.json", "2:1000000000"),
                     ("trie_history_hal.json", "never"), ("trie_coin_absent.json", "unspent:1")] {
        let mut f = trie(n);
        f.expect = Some(bad.into());
        assert!(check_trie_fact(cs, r.undo_from, &f).is_err(), "{n} claiming {bad}");
    }
    // the journey: created in block 170, unspent at 200
    let j: CoinJourney = serde_json::from_slice(&rd("journey_hal_170.json")).unwrap();
    let ok = check_coin_journey(cs, r.undo_from, &j);
    eprintln!("  journey: {ok:?}");
    assert!(ok.is_ok());
}

/// Malformed files, each a FAIL from `verify` and never a panic (a panic is an abort in the CLI, the C ABI and WASM:
/// panic = "abort"). The engine counterparts of the RISC Zero receipt cases (fuzz round 11-17): bytes appended, a
/// journal that is not whole words, a truncated file, and lengths that run past the file.
#[test]
fn malformed_files_are_a_fail() {
    let (file, vk) = (rd("proof.hze"), rd("vk.bin"));
    let img = image().with_engine_key(vk);
    let v = |b: &[u8]| verify(&Inputs { receipt: b, image: img, anchor: None, now: 1_800_000_000 });
    assert!(failed(&v(&file)).is_empty(), "the control verifies");
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut t = file.clone();
    t.push(0);
    cases.push(("a byte appended (the journal is then not whole words)", t));
    let mut t = file.clone();
    t.extend_from_slice(&[0, 0, 0, 0]);
    cases.push(("a word appended", t));
    cases.push(("the last byte cut", file[..file.len() - 1].to_vec()));
    cases.push(("cut inside the proof", file[..file.len() / 2].to_vec()));
    cases.push(("the magic alone", b"HZE1".to_vec()));
    let mut t = file.clone();
    t[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    cases.push(("a proof length past the file", t));
    let mut t = file.clone();
    t[4..8].copy_from_slice(&0u32.to_le_bytes());
    cases.push(("a zero proof length", t));
    let n = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    cases.push(("no journal", file[..8 + n].to_vec()));
    for (what, b) in &cases {
        let r = v(b);
        assert_eq!(r.verdict, Verdict::Fail, "{what}");
        assert!(r.state.is_none(), "{what}: no statement from a file that did not verify");
        eprintln!("  {what}: {}", failed(&r)[0]);
    }
    // the framing reader agrees: a journal not whole words decodes to an error, not a panic
    let mut t = file.clone();
    t.push(0);
    let j = proof_journal(&t).unwrap();
    assert!(decode_journal::<ChainState>(&j).unwrap_err().contains("not whole words"));
}

/// vectors/manifest.json's reference values for a second implementation: the coin forest at mainnet height 200 is the
/// engine proof's verified state.
#[test]
fn manifest_reference_values_are_the_engine_states() {
    let m: serde_json::Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/vectors/manifest.json")).unwrap()).unwrap();
    let u = &m["reference_values"]["coin_log_mainnet_200"];
    let (file, vk) = (rd("proof.hze"), rd("vk.bin"));
    let r = run(&file, image(), &vk, pin());
    let cs = r.state.as_ref().unwrap();
    assert_eq!(cs.utxo_leaves, u["leaves"].as_u64().unwrap());
    assert_eq!(cs.utxo_value, u["utxo_value"].as_u64().unwrap());
    let roots: Vec<(usize, String)> = cs.utxo_roots.iter().enumerate().filter_map(|(i, r)| r.map(|r| (i, hex(&r)))).collect();
    let want: Vec<(usize, String)> = u["roots"].as_object().unwrap().iter().map(|(k, v)| (k.parse().unwrap(), v.as_str().unwrap().to_string())).collect();
    assert_eq!(roots, want);
    assert_eq!(hex(&empty_smt_root()), m["reference_values"]["empty_smt_root"].as_str().unwrap());
}
