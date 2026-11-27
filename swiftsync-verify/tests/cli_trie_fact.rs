//! `swiftsync-verify --trie`: a trie fact with an unknown field is refused, not read with the field ignored (a misspelled
//! `expect` would otherwise turn a claim into a question that cannot fail). The engine chain's proof of mainnet 1..200
//! (tests/data/engine200) and its balance fact for Hal's script; the unmodified fact is the control (it holds, DEMO).
use std::process::Command;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/engine200");

fn run(fact: &serde_json::Value) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("cli_trie_fact_{}_{}", std::process::id(), fact.as_object().unwrap().len()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("fact.json");
    std::fs::write(&p, serde_json::to_vec(fact).unwrap()).unwrap();
    let id = std::fs::read_to_string(format!("{DIR}/image_id")).unwrap();
    // the key's pin, as a reader gives it (review R1-6 #1: a key is never pinned to itself)
    let pin = std::fs::read_to_string(format!("{DIR}/vk.sha256")).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_swiftsync-verify"))
        .args(["--network", "mainnet", "--image", id.trim(), "--vk", &format!("{DIR}/vk.bin"), "--vk-sha256", pin.split_whitespace().next().unwrap(), "--trie"])
        .arg(&p)
        .arg(format!("{DIR}/proof.hze"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr))
}

#[test]
fn an_unknown_field_in_a_trie_fact_is_refused() {
    let f: serde_json::Value = serde_json::from_slice(&std::fs::read(format!("{DIR}/trie_balance_hal.json")).unwrap()).unwrap();
    // control: the real fact holds (DEMO: 200 blocks are below Core's minimum chainwork)
    let (rc, out) = run(&f);
    assert_eq!(rc, 2, "{out}");
    assert!(!out.contains("unknown field"), "{out}");
    // the same fact with its claim under a misspelled key: refused, and the message names the field
    let mut bad = f.clone();
    let claim = bad.as_object_mut().unwrap().remove("expect").unwrap();
    bad.as_object_mut().unwrap().insert("expected".into(), claim);
    let (rc, out) = run(&bad);
    assert_eq!(rc, 1, "{out}");
    assert!(out.contains("unknown field `expected`"), "{out}");
}
