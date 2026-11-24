//! The OpenVM engine as this host's prover (engine/ENGINE.md). Set HAZYNC_ENGINE=<hzengine binary> and
//! HAZYNC_ENGINE_KEYS=<key dir> and `libzync-prove-range` runs every guest execution and proof of a block on the
//! engine instead of RISC Zero: the chunks' signature-collect runs, the chunk proofs, the aggregate's collect run,
//! the trie update and the aggregate, which verifies the chunk, trie and previous-block proofs in-guest. Everything
//! else (blocks, prep, the coin log and trie stores, the BIP30 tree, history) is this host's own code, unchanged.
//!
//! The engine runs as one long-lived `hzengine serve` process (its keys load once). Inputs are the same streams the
//! RISC Zero path writes (crate::sink::Stream); outputs are the same journals.
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

/// A proof made by the engine: its files (P.proof.json, P.journal) and its journal.
#[derive(Clone)]
pub(crate) struct EngineProof {
    pub path: PathBuf,
    pub journal: Vec<u8>,
}

pub(crate) struct Engine {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    work: PathBuf,
    seq: u64,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
/// HAZYNC_ENGINE_GPUS=n (n > 1): n more servers, one per GPU (CUDA_VISIBLE_DEVICES=i), for `prove_many`.
static POOL: Mutex<Vec<Mutex<Engine>>> = Mutex::new(Vec::new());

/// Whether this run proves on the engine (HAZYNC_ENGINE set).
pub(crate) fn active() -> bool {
    std::env::var("HAZYNC_ENGINE").is_ok()
}

fn start(work: &str, gpu: Option<usize>, tag: &str) -> Engine {
    let bin = std::env::var("HAZYNC_ENGINE").expect("HAZYNC_ENGINE");
    let keys = std::env::var("HAZYNC_ENGINE_KEYS").expect("HAZYNC_ENGINE_KEYS: the engine's key directory");
    let mut cmd = Command::new(&bin);
    cmd.args(["serve", "--dir", &keys]).stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(g) = gpu { cmd.env("CUDA_VISIBLE_DEVICES", g.to_string()); }
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("HAZYNC_ENGINE={bin}: {e}"));
    let stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut l = String::new();
    stdout.read_line(&mut l).expect("engine: no ready line");
    assert!(l.contains("\"ready\":true"), "engine did not start: {l}");
    let work = PathBuf::from(work).join("engine").join(tag);
    std::fs::create_dir_all(&work).unwrap();
    println!("  engine: {bin} serving keys {keys}{}; work dir {}", gpu.map(|g| format!(" on GPU {g}")).unwrap_or_default(), work.display());
    Engine { _child: child, stdin, stdout, work, seq: 0 }
}

fn gpus() -> usize {
    std::env::var("HAZYNC_ENGINE_GPUS").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1)
}

/// The engine, started on first use (GPU 0 when HAZYNC_ENGINE_GPUS > 1). `work` holds the streams and its proofs.
pub(crate) fn with<T>(work: &str, f: impl FnOnce(&mut Engine) -> T) -> T {
    let mut g = ENGINE.lock().unwrap();
    if g.is_none() {
        *g = Some(start(work, (gpus() > 1).then_some(0), "s0"));
    }
    f(g.as_mut().unwrap())
}

/// Prove independent jobs (name, stream, children, self_id) on every GPU at once, results in job order: the main
/// server (GPU 0) and HAZYNC_ENGINE_GPUS - 1 more. One GPU: in order on the main server.
pub(crate) fn prove_many(work: &str, jobs: Vec<(String, Vec<u8>, Vec<EngineProof>, bool)>) -> Vec<EngineProof> {
    let n = gpus();
    if n == 1 || jobs.len() < 2 {
        return jobs.into_iter().map(|(name, st, ch, sid)| {
            let c: Vec<&EngineProof> = ch.iter().collect();
            with(work, |e| e.prove(&name, &st, &c, sid))
        }).collect();
    }
    with(work, |_| ()); // the main server exists (GPU 0)
    {
        let mut p = POOL.lock().unwrap();
        while p.len() < n - 1 {
            let g = p.len() + 1;
            p.push(Mutex::new(start(work, Some(g), &format!("s{g}"))));
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out: Mutex<Vec<Option<EngineProof>>> = Mutex::new(vec![None; jobs.len()]);
    let pool = POOL.lock().unwrap();
    std::thread::scope(|sc| {
        for slot in 0..n {
            let (jobs, next, out, pool) = (&jobs, &next, &out, &pool);
            sc.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((name, st, ch, sid)) = jobs.get(i) else { break };
                let c: Vec<&EngineProof> = ch.iter().collect();
                let p = if slot == 0 { with(work, |e| e.prove(name, st, &c, *sid)) } else { pool[slot - 1].lock().unwrap().prove(name, st, &c, *sid) };
                out.lock().unwrap()[i] = Some(p);
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|p| p.expect("engine: a job produced no proof")).collect()
}

pub(crate) struct Exec {
    /// env::exit's code when the guest ended that way (a collect run), else None (committed a journal).
    pub exit: Option<u32>,
    pub log: String,
    pub journal: Option<Vec<u8>>,
}

impl Engine {
    fn req(&mut self, v: serde_json::Value) -> serde_json::Value {
        writeln!(self.stdin, "{v}").and_then(|_| self.stdin.flush()).expect("engine: request");
        let mut l = String::new();
        self.stdout.read_line(&mut l).expect("engine: reply");
        let r: serde_json::Value = serde_json::from_str(&l).unwrap_or_else(|e| panic!("engine: bad reply {l:?}: {e}"));
        if r["ok"] != true {
            panic!("engine: {} failed: {}", v["op"], r["error"]);
        }
        r
    }

    fn file(&mut self, name: &str, ext: &str) -> PathBuf {
        self.seq += 1;
        self.work.join(format!("{:05}_{name}.{ext}", self.seq))
    }

    /// Execute (no proof). `children` are verified in-guest in env::verify order; `self_id` puts the engine's image
    /// id where the stream carries this host's METHOD_ID (bytes 4..36: an aggregate or a join).
    pub(crate) fn execute(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool) -> Exec {
        let input = self.file(name, "in");
        std::fs::write(&input, stream).unwrap();
        let (log, journal) = (input.with_extension("log"), input.with_extension("journal"));
        let ch: Vec<String> = children.iter().map(|c| c.path.display().to_string()).collect();
        let r = self.req(serde_json::json!({"op": "execute", "input": input, "children": ch, "self_id": self_id,
                                            "journal": journal, "log": log}));
        let exit = r["exit"].as_str().map(|c| c.parse().expect("engine: exit code"));
        let out = Exec {
            exit,
            log: std::fs::read_to_string(&log).unwrap_or_default(),
            journal: if exit.is_none() { Some(std::fs::read(&journal).expect("engine: journal")) } else { None },
        };
        let _ = std::fs::remove_file(&input);
        out
    }

    /// Prove; the proof is kept in the work dir under `name`.
    pub(crate) fn prove(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool) -> EngineProof {
        let input = self.file(name, "in");
        std::fs::write(&input, stream).unwrap();
        let out = input.with_extension("");
        let ch: Vec<String> = children.iter().map(|c| c.path.display().to_string()).collect();
        let r = self.req(serde_json::json!({"op": "prove", "input": input, "children": ch, "self_id": self_id, "out": out}));
        println!("  engine: {name} proved in {:.1} s ({} children verified in-guest)", r["secs"].as_f64().unwrap_or(0.0), children.len());
        let journal = std::fs::read(with_ext(&out, "journal")).expect("engine: proof journal");
        let _ = std::fs::remove_file(&input);
        EngineProof { path: out, journal }
    }
}

pub(crate) fn with_ext(p: &Path, ext: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    s.into()
}

/// Copy an engine proof to `dst` (P.proof.json + P.journal), e.g. next to the run's other per-block files.
pub(crate) fn keep(p: &EngineProof, dst: &str) -> EngineProof {
    let d = PathBuf::from(dst);
    for ext in ["proof.json", "journal"] {
        std::fs::copy(with_ext(&p.path, ext), with_ext(&d, ext)).unwrap_or_else(|e| panic!("engine: keep {dst}.{ext}: {e}"));
    }
    EngineProof { path: d, journal: p.journal.clone() }
}

/// What write_aggregate_env_ex needs from a child proof: its journal, and the RISC Zero assumption if it is one.
pub(crate) trait Proven {
    fn journal_bytes(&self) -> &[u8];
    fn assumption(&self) -> Option<risc0_zkvm::Receipt>;
}

impl Proven for risc0_zkvm::Receipt {
    fn journal_bytes(&self) -> &[u8] { &self.journal.bytes }
    fn assumption(&self) -> Option<risc0_zkvm::Receipt> { Some(self.clone()) }
}

impl Proven for EngineProof {
    fn journal_bytes(&self) -> &[u8] { &self.journal }
    fn assumption(&self) -> Option<risc0_zkvm::Receipt> { None }
}

/// The trie update's engine proof for the aggregate being built (the engine's counterpart of lz::TRIE_RCPT).
pub(crate) static TRIE: Mutex<Option<EngineProof>> = Mutex::new(None);

/// A ChainState from a journal (RISC Zero's serde words, as both backends commit it).
pub(crate) fn decode_state(j: &[u8]) -> crate::ChainState {
    assert!(j.len() % 4 == 0, "engine: journal is not whole words");
    let w: Vec<u32> = j.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    risc0_zkvm::serde::from_slice(&w).expect("engine: ChainState")
}

/// The engine image id as the guest commits it (ChainState.self_id): sha256(exe || vm) of `<keys>/image.json`'s
/// commitments, as 8 little-endian words.
pub(crate) fn image_id_words(keys: &str) -> [u32; 8] {
    use sha2::Digest;
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(format!("{keys}/image.json"))
        .unwrap_or_else(|e| panic!("{keys}/image.json: {e} (the engine records it with its first proof)"))).expect("image.json");
    let mut pre = hex_bytes(v["exe"].as_str().expect("image.json exe"));
    pre.extend(hex_bytes(v["vm"].as_str().expect("image.json vm")));
    assert!(pre.len() == 64, "image.json: two 32-byte commitments");
    let d = sha2::Sha256::digest(&pre);
    core::array::from_fn(|i| u32::from_le_bytes(d[4 * i..4 * i + 4].try_into().unwrap()))
}

fn hex_bytes(h: &str) -> Vec<u8> {
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).expect("hex")).collect()
}
