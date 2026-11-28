//! Rust side of the transaction-parser differential test: reads the corpus ("T <hex>" / "P <hex>") on stdin and prints,
//! per line, what each Rust parser on the soundness path makes of it (compare.py lines this up with Core's harness).
//!   agg_lite.rs tx_io / prevout_fields (the core guest's trie keys and undo records), libzync facts::parse_tx (the
//!   verifiers' transaction facts), hints.rs / utreexo_roots.rs parse_tx (the zAPP guests' stripped-form reader).
//! `--lenient` / `--nomarker` swap in the positive controls' readers (build.rs).
#![allow(dead_code)]
use sha2::{Digest, Sha256};
use std::io::{self, BufRead, Write};

type H = [u8; 32];
fn dsha(b: &[u8]) -> H { Sha256::digest(Sha256::digest(b)).into() }

mod strict { include!(concat!(env!("OUT_DIR"), "/agg.rs")); }
mod lenient { include!(concat!(env!("OUT_DIR"), "/agg_lenient.rs")); }
mod nomarker { include!(concat!(env!("OUT_DIR"), "/agg_nomarker.rs")); }
mod hints { use super::{dsha, H}; use libzync::facts; include!(concat!(env!("OUT_DIR"), "/hints.rs"));
    pub fn run(b: &[u8]) -> Option<(usize, H, Vec<(Vec<u8>, u32)>, Vec<(u64, Vec<u8>)>)> {
        let mut o = 0;
        let t = parse_tx(b, &mut o).ok()?;
        Some((o, t.txid, t.inputs.iter().map(|(a, v)| (a.to_vec(), *v)).collect(), t.outputs.iter().map(|(v, s)| (*v, s.to_vec())).collect()))
    } }
mod utreexo { use super::{dsha, H}; use libzync::facts; include!(concat!(env!("OUT_DIR"), "/utreexo.rs"));
    pub fn run(b: &[u8]) -> Option<(usize, H, Vec<(Vec<u8>, u32)>, Vec<(u64, Vec<u8>)>)> {
        let mut o = 0;
        let t = parse_tx(b, &mut o).ok()?;
        Some((o, t.txid, t.inputs.iter().map(|(a, v)| (a.to_vec(), *v)).collect(), t.outputs.iter().map(|(v, s)| (*v, s.to_vec())).collect()))
    } }

fn hx(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 { return None; }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}
fn list<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
    if v.is_empty() { ".".into() } else { v.iter().map(f).collect::<Vec<_>>().join(",") }
}
fn script(s: &[u8]) -> String { if s.is_empty() { ".".into() } else { hx(s) } }
fn stripped(name: &str, r: Option<(usize, H, Vec<(Vec<u8>, u32)>, Vec<(u64, Vec<u8>)>)>) -> String {
    match r {
        None => format!("{name} err"),
        Some((o, txid, ins, outs)) => format!("{name} ok {o} {} {} {} {} {}", hx(&txid), ins.len(),
            list(&ins, |(t, v)| format!("{}:{v}", hx(t))), outs.len(), list(&outs, |(v, s)| format!("{v}:{}", script(s)))),
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let (tx_io, prevout_fields): (fn(&[u8]) -> _, fn(&[u8]) -> _) = match arg.as_str() {
        "--lenient" => (lenient::tx_io, lenient::prevout_fields),
        "--nomarker" => (nomarker::tx_io, nomarker::prevout_fields),
        "" => (strict::tx_io, strict::prevout_fields),
        a => panic!("unknown argument {a}"),
    };
    let stdout = io::stdout();
    let mut w = io::BufWriter::new(stdout.lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.len() < 2 { continue; }
        let Some(b) = unhex(&line[2..]) else { writeln!(w, "bad").unwrap(); continue };
        match line.as_bytes()[0] {
            b'T' => {
                // tx_io: the outpoints, and each output's script by the range it returns
                match tx_io(&b) {
                    None => writeln!(w, "T io err").unwrap(),
                    Some((ins, outs)) => {
                        let scripts: Option<Vec<&[u8]>> = outs.iter().map(|(o, l)| b.get(*o..o.checked_add(*l)?)).collect();
                        match scripts {
                            None => writeln!(w, "T io ok {} {} {} range-outside", ins.len(), list(&ins, |(t, v)| format!("{}:{v}", hx(t))), outs.len()).unwrap(),
                            Some(s) => writeln!(w, "T io ok {} {} {} {}", ins.len(), list(&ins, |(t, v)| format!("{}:{v}", hx(t))), s.len(), list(&s, |x| script(x))).unwrap(),
                        }
                    }
                }
                let mut o = 0;
                match libzync::facts::parse_tx(&b, &mut o) {
                    None => writeln!(w, "T facts err").unwrap(),
                    Some(t) => writeln!(w, "T facts ok {o} {} {} {} {} {}", hx(&dsha(&t.stripped)), t.inputs.len(),
                        list(&t.inputs, |(h, v)| format!("{}:{v}", hx(h))), t.outputs.len(), list(&t.outputs, |(v, s)| format!("{v}:{}", script(s)))).unwrap(),
                }
                writeln!(w, "T {}", stripped("hints", hints::run(&b))).unwrap();
                writeln!(w, "T {}", stripped("utreexo", utreexo::run(&b))).unwrap();
            }
            b'P' => match prevout_fields(&b) {
                None => writeln!(w, "P err").unwrap(),
                Some(v) => {
                    let s: Option<Vec<String>> = v.iter().map(|(val, o, l)| Some(format!("{val}:{}", script(b.get(*o..o.checked_add(*l)?)?)))).collect();
                    match s {
                        None => writeln!(w, "P ok {} range-outside", v.len()).unwrap(),
                        Some(s) => writeln!(w, "P ok {} {}", v.len(), if s.is_empty() { ".".into() } else { s.join(",") }).unwrap(),
                    }
                }
            },
            _ => writeln!(w, "bad").unwrap(),
        }
    }
}
