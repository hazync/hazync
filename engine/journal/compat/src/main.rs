use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
enum E { A, B(u32), C { x: u64, y: Vec<u8> }, D(i64, bool) }
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
struct S { a: u8, b: u16, c: u32, d: u64, e: i8, f: i16, g: i32, h: i64, i: bool, j: Option<u32>, k: Option<[u8; 32]>,
  l: Vec<u8>, m: Vec<u32>, n: String, o: [u32; 8], p: (u8, u64, bool), q: Vec<E>, r: Vec<Option<Vec<u8>>>, s: u128, t: f32, u: char, v: () }
fn main() {
  let mut n = 0;
  for seed in 0u64..2000 {
    let r = |k: u64| seed.wrapping_mul(0x9E3779B97F4A7C15).rotate_left(k as u32 % 63) ^ k;
    let len = (r(1) % 9) as usize;
    let s = S { a: r(2) as u8, b: r(3) as u16, c: r(4) as u32, d: r(5), e: r(6) as i8, f: r(7) as i16, g: r(8) as i32, h: r(9) as i64,
      i: r(10) & 1 == 1, j: if r(11) & 1 == 0 { None } else { Some(r(12) as u32) }, k: if r(13) & 1 == 0 { None } else { Some([r(14) as u8; 32]) },
      l: (0..len).map(|i| r(20 + i as u64) as u8).collect(), m: (0..len).map(|i| r(40 + i as u64) as u32).collect(),
      n: "héllo☃".chars().take(len).collect(), o: std::array::from_fn(|i| r(60 + i as u64) as u32), p: (r(70) as u8, r(71), r(72) & 2 == 0),
      q: vec![E::A, E::B(r(80) as u32), E::C { x: r(81), y: vec![1; len] }, E::D(r(82) as i64, true)][..(len % 5)].to_vec(),
      r: vec![None, Some(vec![r(90) as u8; len % 7])], s: ((r(91) as u128) << 64) | r(92) as u128, t: (r(93) % 1000) as f32 / 7.0, u: 'ß', v: () };
    let a = risc0_zkvm::serde::to_vec(&s).unwrap();
    let b = hazync_journal::serde::to_vec(&s).unwrap();
    assert_eq!(a, b, "seed {seed}");
    let bytes: Vec<u8> = a.iter().flat_map(|w| w.to_le_bytes()).collect();
    let x: S = risc0_zkvm::Journal::new(bytes.clone()).decode().unwrap();
    let y: S = hazync_journal::Journal::new(bytes).decode().unwrap();
    assert_eq!(x, y); assert_eq!(y, s); n += 1;
  }
  // the real statement type, through both
  let st = swiftsync_verify::genesis_anchor(swiftsync_verify::Image::by_network("mainnet").unwrap());
  assert_eq!(risc0_zkvm::serde::to_vec(&st).unwrap(), hazync_journal::serde::to_vec(&st).unwrap());
  let d = risc0_zkvm::sha::Digest::from_bytes([7u8; 32].map(|_| 0).iter().enumerate().map(|(i, _)| i as u8 * 9).collect::<Vec<_>>().try_into().unwrap());
  assert_eq!(swiftsync_verify::words(&d), hazync_journal::id_words(d.as_bytes().try_into().unwrap()));
  assert_eq!(hazync_journal::id_bytes(&swiftsync_verify::words(&d)), <[u8;32]>::try_from(d.as_bytes()).unwrap());
  println!("COMPAT OK: {n} random values + ChainState + id words, byte-identical to risc0-zkvm 3.1.0");
}
