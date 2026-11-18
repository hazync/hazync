//! hints-crate-ref write <indices.json> <H> <out>   : the file the `hintsfile` crate 0.2.0 writes for these lists
//! hints-crate-ref read <file> <out indices.json>    : the lists the crate reads back
use hintsfile::{EliasFano, Hintsfile, HintsfileBuilder};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a[0].as_str() {
        "write" => {
            let lists: Vec<Vec<u32>> = serde_json::from_slice(&std::fs::read(&a[1]).unwrap()).unwrap();
            let h: u32 = a[2].parse().unwrap();
            assert_eq!(lists.len(), h as usize);
            let mut out = Vec::new();
            {
                let mut b = HintsfileBuilder::new(&mut out).initialize(h).unwrap();
                for l in &lists {
                    b.append(EliasFano::compress(l)).unwrap();
                }
                b.finish().unwrap();
            }
            std::fs::write(&a[3], &out).unwrap();
            println!("CRATE_WRITE {} bytes", out.len());
        }
        "read" => {
            let mut f = std::fs::File::open(&a[1]).unwrap();
            let mut hf = Hintsfile::from_reader(&mut f).unwrap();
            let h = hf.stop_height();
            let lists: Vec<Vec<u32>> = (1..=h).map(|x| hf.take_indices(x).unwrap()).collect();
            std::fs::write(&a[2], serde_json::to_vec(&lists).unwrap()).unwrap();
            println!("CRATE_READ stop_height {h} unspent {}", lists.iter().map(|l| l.len()).sum::<usize>());
        }
        _ => panic!("write | read"),
    }
}
