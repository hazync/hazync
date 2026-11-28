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
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    work: PathBuf,
    seq: u64,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
/// The other servers `prove_many` uses: HAZYNC_ENGINE_GPUS x share() in all, the main one included, spread over the
/// GPUs (CUDA_VISIBLE_DEVICES=i).
static POOL: Mutex<Vec<Mutex<Engine>>> = Mutex::new(Vec::new());

/// Whether this run proves on the engine (HAZYNC_ENGINE set).
pub(crate) fn active() -> bool {
    std::env::var("HAZYNC_ENGINE").is_ok()
}

/// The engine's DEV MODE (HAZYNC_ENGINE_DEV=1), the counterpart of RISC Zero's RISC0_DEV_MODE for test harnesses (the
/// core-rules rig, CUTOVER C8): every "proof" is an execution, nothing is proved. Allowed only with a METERING image
/// (a keys dir holding BENCH_NOVERIFY: the guest feature bench-noverify, whose env::verify accepts a child without a
/// proof), so the guest's own checks all run while its in-guest verification is skipped, and nothing it makes can pass
/// as a proof: no proof file exists, and a metering image's public values are tagged (engine.rs bench_pvs), so no
/// reader accepts any proof of it either. A dev "proof" is its journal and a P.proof.json saying it is none.
pub(crate) fn dev() -> bool {
    static D: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *D.get_or_init(|| {
        if std::env::var("HAZYNC_ENGINE_DEV").ok().as_deref() != Some("1") { return false; }
        let keys = std::env::var("HAZYNC_ENGINE_KEYS").expect("HAZYNC_ENGINE_DEV=1 needs HAZYNC_ENGINE_KEYS");
        assert!(Path::new(&keys).join("BENCH_NOVERIFY").exists(),
                "HAZYNC_ENGINE_DEV=1 runs only a METERING image (a keys dir with BENCH_NOVERIFY, guest feature bench-noverify); {keys} is not one");
        println!("  engine: DEV MODE (HAZYNC_ENGINE_DEV=1, metering image {keys}): executions only, nothing is proved");
        true
    })
}

fn start(work: &str, gpu: Option<usize>, tag: &str) -> Engine {
    let bin = std::env::var("HAZYNC_ENGINE").expect("HAZYNC_ENGINE");
    let keys = std::env::var("HAZYNC_ENGINE_KEYS").expect("HAZYNC_ENGINE_KEYS: the engine's key directory");
    let mut cmd = Command::new(&bin);
    cmd.args(["serve", "--dir", &keys]).stdin(Stdio::piped()).stdout(Stdio::piped());
    if let Some(g) = gpu { cmd.env("CUDA_VISIBLE_DEVICES", g.to_string()); }
    cmd.env("HZ_GPU_SHARE", share().to_string());
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("HAZYNC_ENGINE={bin}: {e}"));
    let stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut l = String::new();
    stdout.read_line(&mut l).expect("engine: no ready line");
    assert!(l.contains("\"ready\":true"), "engine did not start: {l}");
    let work = PathBuf::from(work).join("engine").join(tag);
    std::fs::create_dir_all(&work).unwrap();
    println!("  engine: {bin} serving keys {keys}{}; work dir {}", gpu.map(|g| format!(" on GPU {g}")).unwrap_or_default(), work.display());
    Engine { child, stdin, stdout, work, seq: 0 }
}

fn gpus() -> usize {
    std::env::var("HAZYNC_ENGINE_GPUS").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1)
}

/// Provers per GPU: HAZYNC_ENGINE_PER_GPU, else the engine's own default for this card (`hzengine gpu-share`: 3 on
/// 40 GiB or more, measured on an A6000 in engine/results/gpu_a6000_tuning; else 1). Each server is told the share,
/// which sizes its segment cap so the provers on one card fit together.
fn share() -> usize {
    static S: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *S.get_or_init(|| {
        if let Some(n) = std::env::var("HAZYNC_ENGINE_PER_GPU").ok().and_then(|v| v.parse::<usize>().ok()) {
            return n.max(1);
        }
        let bin = std::env::var("HAZYNC_ENGINE").expect("HAZYNC_ENGINE");
        let mut c = Command::new(&bin);
        c.arg("gpu-share");
        if gpus() > 1 { c.env("CUDA_VISIBLE_DEVICES", "0"); }
        let o = c.output().unwrap_or_else(|e| panic!("{bin} gpu-share: {e}"));
        String::from_utf8_lossy(&o.stdout).trim().parse::<usize>().unwrap_or(1).max(1)
    })
}

/// The engine, started on first use (GPU 0 when HAZYNC_ENGINE_GPUS > 1). `work` holds the streams and its proofs.
pub(crate) fn with<T>(work: &str, f: impl FnOnce(&mut Engine) -> T) -> T {
    let mut g = ENGINE.lock().unwrap();
    if g.is_none() {
        *g = Some(start(work, (gpus() > 1).then_some(0), "s0"));
    }
    f(g.as_mut().unwrap())
}

/// Prove independent jobs (name, stream, children, self_id) on every server at once, results in job order: the main
/// server (GPU 0) and HAZYNC_ENGINE_GPUS x share() - 1 more, server k on GPU k mod GPUs. One server: in order.
pub(crate) type Job = (String, Vec<u8>, Vec<EngineProof>, bool, Option<Vec<u8>>);

pub(crate) fn prove_many(work: &str, jobs: Vec<Job>) -> Vec<EngineProof> {
    let (g, n) = (gpus(), gpus() * share());
    if n == 1 || jobs.len() < 2 {
        return jobs.into_iter().map(|(name, st, ch, sid, known)| {
            let c: Vec<&EngineProof> = ch.iter().collect();
            with(work, |e| e.prove_known(&name, &st, &c, sid, known.as_deref()))
        }).collect();
    }
    with(work, |_| ()); // the main server exists (GPU 0)
    {
        let mut p = POOL.lock().unwrap();
        while p.len() < n - 1 {
            let k = p.len() + 1;
            p.push(Mutex::new(start(work, (g > 1).then_some(k % g), &format!("s{k}"))));
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
                let Some((name, st, ch, sid, known)) = jobs.get(i) else { break };
                let c: Vec<&EngineProof> = ch.iter().collect();
                let p = if slot == 0 { with(work, |e| e.prove_known(name, st, &c, *sid, known.as_deref())) }
                        else { pool[slot - 1].lock().unwrap().prove_known(name, st, &c, *sid, known.as_deref()) };
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
    /// The engine's cost of the run (its metered cells; the scheduler's measure of a chunk's size).
    pub cost: u64,
}

impl Engine {
    fn req(&mut self, v: serde_json::Value) -> serde_json::Value {
        let op = v["op"].clone();
        self.try_req(v).unwrap_or_else(|e| panic!("engine: {op} failed: {e}"))
    }

    /// One request; a reply that is not ok is an Err carrying the engine's error.
    fn try_req(&mut self, v: serde_json::Value) -> Result<serde_json::Value, String> {
        // a server that died (a panic, the OOM killer) closes its pipes: say so, with its exit status, rather than
        // reporting an empty "bad reply" (its own last words are on stderr, above)
        if writeln!(self.stdin, "{v}").and_then(|_| self.stdin.flush()).is_err() {
            panic!("engine: the server is gone ({}); see its stderr above", self.exit_status());
        }
        let mut l = String::new();
        if self.stdout.read_line(&mut l).map(|n| n == 0).unwrap_or(true) {
            panic!("engine: the server ended without replying to {} ({}); see its stderr above", v["op"], self.exit_status());
        }
        let r: serde_json::Value = serde_json::from_str(&l).unwrap_or_else(|e| panic!("engine: bad reply {l:?}: {e}"));
        if r["ok"] != true {
            return Err(r["error"].to_string());
        }
        Ok(r)
    }

    /// The server's exit status once it has ended (waits briefly: its pipes close just before it is reaped).
    fn exit_status(&mut self) -> String {
        for _ in 0..50 {
            match self.child.try_wait() {
                Ok(Some(st)) => return st.to_string(),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(e) => return format!("its status unknown: {e}"),
            }
        }
        "still running, its pipes closed".into()
    }
    fn file(&mut self, name: &str, ext: &str) -> PathBuf {
        self.seq += 1;
        self.work.join(format!("{:05}_{name}.{ext}", self.seq))
    }

    /// Execute (no proof). `children` are verified in-guest in env::verify order; `self_id` puts the engine's image
    /// id where the stream carries this host's METHOD_ID (bytes 4..36: an aggregate or a join).
    pub(crate) fn execute(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool) -> Exec {
        self.execute_ex(name, stream, children, self_id, false)
    }

    /// `dry`: a collect run that skips in-guest verification (takes no proofs) and can never commit.
    pub(crate) fn execute_ex(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool, dry: bool) -> Exec {
        // dev mode: the children are dev "proofs" (journals only); the metering guest's env::verify takes none
        let children: &[&EngineProof] = if dev() { &[] } else { children };
        let input = self.file(name, "in");
        std::fs::write(&input, stream).unwrap();
        let (log, journal) = (input.with_extension("log"), input.with_extension("journal"));
        let ch: Vec<String> = children.iter().map(|c| c.path.display().to_string()).collect();
        // A failed run panics with the end of the guest's log, as try_execute reports it: the guest's own verdict
        // (e.g. "agg-lite: block INVALID ...", then its refusal to commit) is there, where RISC Zero's executor prints
        // guest output straight to this process's output (the core-rules rig reads it; 26.11.2026).
        let r = self.try_req(serde_json::json!({"op": "execute", "input": input, "children": ch, "self_id": self_id,
                                                "dry": dry, "journal": journal, "log": log}))
            .unwrap_or_else(|e| {
                let text = std::fs::read_to_string(&log).unwrap_or_default();
                let tail: Vec<&str> = text.lines().rev().take(8).collect();
                panic!("engine: execute of {name} failed: {e}; guest log ends: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | "))
            });
        let exit = r["exit"].as_str().map(|c| c.parse().expect("engine: exit code"));
        let out = Exec {
            exit,
            log: std::fs::read_to_string(&log).unwrap_or_default(),
            journal: if exit.is_none() { Some(std::fs::read(&journal).expect("engine: journal")) } else { None },
            cost: r["cost"].as_u64().unwrap_or(0),
        };
        let _ = std::fs::remove_file(&input);
        out
    }

    /// `execute` with a failed run as Err (instead of a panic): the engine's error, then the last lines of the guest's
    /// log, where an out-of-memory guest says so (lz::is_guest_oom reads it; #794's re-split).
    pub(crate) fn try_execute(&mut self, name: &str, stream: &[u8]) -> Result<Exec, String> {
        let input = self.file(name, "in");
        std::fs::write(&input, stream).unwrap();
        let (log, journal) = (input.with_extension("log"), input.with_extension("journal"));
        let r = self.try_req(serde_json::json!({"op": "execute", "input": input, "children": Vec::<String>::new(),
                                                "self_id": false, "dry": false, "journal": journal, "log": log}));
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let _ = std::fs::remove_file(&input);
        let r = r.map_err(|e| {
            let tail: Vec<&str> = text.lines().rev().take(5).collect();
            format!("engine execute of {name} failed: {e}; guest log ends: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" | "))
        })?;
        let exit = r["exit"].as_str().map(|c| c.parse().expect("engine: exit code"));
        Ok(Exec {
            exit,
            log: text,
            journal: if exit.is_none() { Some(std::fs::read(&journal).expect("engine: journal")) } else { None },
            cost: r["cost"].as_u64().unwrap_or(0),
        })
    }

    /// Prove; the proof is kept in the work dir under `name`.
    pub(crate) fn prove(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool) -> EngineProof {
        self.prove_known(name, stream, children, self_id, None)
    }

    /// `known`: the journal an execution of this same stream committed; the engine then skips its own execute pass
    /// and checks the proof against it.
    pub(crate) fn prove_known(&mut self, name: &str, stream: &[u8], children: &[&EngineProof], self_id: bool, known: Option<&[u8]>) -> EngineProof {
        if dev() {
            // an execution in place of the proof: its journal, and a P.proof.json that says it is none
            let x = self.execute_ex(name, stream, children, self_id, false);
            let journal = x.journal.unwrap_or_else(|| panic!("engine dev: {name} ended with exit {:?}, no journal", x.exit));
            if let Some(k) = known { assert_eq!(k, &journal[..], "engine dev: {name}: the execution's journal is not the known one"); }
            self.seq += 1;
            let out = self.work.join(format!("{:05}_{name}", self.seq));
            std::fs::write(with_ext(&out, "journal"), &journal).unwrap();
            std::fs::write(with_ext(&out, "proof.json"), "\"HAZYNC_ENGINE_DEV: an execution, not a proof\"\n").unwrap();
            println!("  engine: {name} EXECUTED (dev mode: no proof)");
            return EngineProof { path: out, journal };
        }
        let input = self.file(name, "in");
        std::fs::write(&input, stream).unwrap();
        let out = input.with_extension("");
        let kj = known.map(|j| { let p = input.with_extension("known"); std::fs::write(&p, j).unwrap(); p });
        let ch: Vec<String> = children.iter().map(|c| c.path.display().to_string()).collect();
        let r = self.req(serde_json::json!({"op": "prove", "input": input, "children": ch, "self_id": self_id, "journal": kj, "out": out}));
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

/// A journal from an execution: what a collect aggregate's stream carries for a chunk not yet proved.
pub(crate) struct JournalOnly(pub Vec<u8>);

impl Proven for JournalOnly {
    fn journal_bytes(&self) -> &[u8] { &self.0 }
    fn assumption(&self) -> Option<risc0_zkvm::Receipt> { None }
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
    hazync_journal::serde::from_slice(&w).expect("engine: ChainState")
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

impl Engine {
    /// `src` (a P.proof.json + P.journal pair, or HZE1) as one HZE1 file at `dst`: the board's and the readers' proof
    /// file, checked by verify-core before it is written (engine/STAGE3.md 3.2).
    pub(crate) fn pack(&mut self, src: &Path, dst: &Path) {
        let r = self.req(serde_json::json!({"op": "pack", "input": src, "out": dst}));
        println!("  engine: packed {} -> {} ({} bytes, image {})", src.display(), dst.display(), r["bytes"], r["image_id"].as_str().unwrap_or("?"));
    }
}

/// An engine proof at `p`: an HZE1 file (the board's), or a P.proof.json + P.journal pair. Either can be a child.
pub(crate) fn read_any(p: &str) -> EngineProof {
    if let Ok(f) = std::fs::read(p) {
        if hazync_engine_verify::is_engine_proof(&f) {
            let (_, j) = hazync_engine_verify::unpack(&f).unwrap_or_else(|e| panic!("{p}: {e}"));
            return EngineProof { path: p.into(), journal: j.to_vec() };
        }
    }
    EngineProof { path: p.into(), journal: std::fs::read(format!("{p}.journal")).unwrap_or_else(|e| panic!("{p}.journal: {e}")) }
}

/// Does `out` name a one-file proof (the board's `.bin`, or `.hze`)? Then a command writes HZE1 there, else a pair.
pub(crate) fn wants_hze(out: &str) -> bool {
    out.ends_with(".bin") || out.ends_with(".hze")
}

/// The board's engine key (HAZYNC_ENGINE_VK = the key file `hzengine pack --key` writes, HAZYNC_ENGINE_VK_PIN = its
/// SHA-256, hex): None when this host is not set up as an engine board. A key whose bytes are not the pin is an Err.
pub(crate) fn board_key() -> Option<Result<(Vec<u8>, [u8; 32]), String>> {
    let path = std::env::var("HAZYNC_ENGINE_VK").ok()?;
    Some((|| {
        let pin_hex = std::env::var("HAZYNC_ENGINE_VK_PIN").map_err(|_| "HAZYNC_ENGINE_VK is set without HAZYNC_ENGINE_VK_PIN (the key's SHA-256)".to_string())?;
        let pin: [u8; 32] = hex_bytes(pin_hex.trim()).try_into().map_err(|_| "HAZYNC_ENGINE_VK_PIN is not 64 hex digits".to_string())?;
        let key = std::fs::read(&path).map_err(|e| format!("HAZYNC_ENGINE_VK={path}: {e}"))?;
        use sha2::Digest;
        if sha2::Sha256::digest(&key)[..] != pin[..] { return Err(format!("HAZYNC_ENGINE_VK={path} is not the pinned key (its SHA-256 is not HAZYNC_ENGINE_VK_PIN)")); }
        Ok((key, pin))
    })())
}

/// The engine image id this host proves or verifies as, 8 words: the board key's image, else HAZYNC_ENGINE_KEYS'
/// image.json; None for a RISC Zero host.
pub(crate) fn engine_image_words() -> Option<Result<[u32; 8], String>> {
    if let Some(k) = board_key() {
        return Some(k.and_then(|(key, _)| {
            let vk = hazync_engine_verify::decode_vk(&key).map_err(|e| format!("HAZYNC_ENGINE_VK does not decode: {e}"))?;
            let id = hazync_engine_verify::image_id_of(&vk);
            Ok(core::array::from_fn(|i| u32::from_le_bytes(id[4 * i..4 * i + 4].try_into().unwrap())))
        }));
    }
    let keys = std::env::var("HAZYNC_ENGINE_KEYS").ok()?;
    Some(Ok(image_id_words(&keys)))
}

/// [`engine_image_words`] once per process; a host set up for the engine whose key or image cannot be read stops.
pub(crate) fn engine_image_cached() -> Option<[u32; 8]> {
    static I: std::sync::OnceLock<Option<[u32; 8]>> = std::sync::OnceLock::new();
    *I.get_or_init(|| engine_image_words().map(|r| r.unwrap_or_else(|e| panic!("engine image: {e}"))))
}
