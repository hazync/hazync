//! `hzengine serve --dir <keys>`: the engine as a long-lived process for the product host's prover backend
//! (prover/host/src/engine_backend.rs). The SDK loads once (~25 s); then one JSON request per stdin line, one JSON
//! reply per stdout line. Requests:
//!   {"op":"execute","input":P,"children":[..],"self_id":b,"dry":b,"journal":P?,"log":P?}
//!   {"op":"prove","input":P,"children":[..],"self_id":b,"journal":P?,"out":P}   journal = known (skip emit pass)
//!   {"op":"image"}                                  -> {"ok":true,"image_id":hex}  (needs image.json)
//! Replies: {"ok":true,...} or {"ok":false,"error":"..."}. Guest output never reaches stdout (it is captured), and
//! progress goes to stderr, so stdout carries only replies.
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use eyre::Result;
use serde::Deserialize;

use crate::engine::Engine;

#[derive(Deserialize)]
struct Req {
    op: String,
    #[serde(default)]
    input: Option<PathBuf>,
    #[serde(default)]
    children: Vec<PathBuf>,
    #[serde(default)]
    self_id: bool,
    #[serde(default)]
    dry: bool,
    #[serde(default)]
    journal: Option<PathBuf>,
    #[serde(default)]
    log: Option<PathBuf>,
    #[serde(default)]
    out: Option<PathBuf>,
    #[serde(default)]
    lo: usize,
    #[serde(default)]
    hi: usize,
}

pub fn serve(dir: &Path) -> Result<()> {
    // Replies get their own descriptor (the original stdout); fd 1 becomes stderr, so the SDK's tracing and any other
    // print can never corrupt the reply stream. (capture.rs still redirects fd 1 around a guest execution.)
    use std::os::fd::FromRawFd;
    std::io::stdout().flush().ok();
    // SAFETY: plain fd juggling on this process's own descriptors, before any thread is started here.
    let reply_fd = unsafe { libc::dup(1) };
    assert!(reply_fd >= 0 && unsafe { libc::dup2(2, 1) } >= 0, "serve: fd setup");
    let mut stdout = unsafe { std::fs::File::from_raw_fd(reply_fd) };
    let e = Engine::load(dir)?;
    let stdin = std::io::stdin();
    writeln!(stdout, "{}", serde_json::json!({"ok": true, "ready": true}))?;
    stdout.flush()?;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Req>(&line) {
            Err(err) => serde_json::json!({"ok": false, "error": format!("bad request: {err}")}),
            Ok(r) => match handle(&e, dir, &r) {
                Ok(v) => v,
                Err(err) => serde_json::json!({"ok": false, "error": format!("{err:#}")}),
            },
        };
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(e: &Engine, dir: &Path, r: &Req) -> Result<serde_json::Value> {
    let t = std::time::Instant::now();
    match r.op.as_str() {
        "execute" => {
            let input = r.input.as_ref().ok_or_else(|| eyre::eyre!("execute: input"))?;
            let o = crate::r0::execute_with(e, dir, input, &r.children, r.self_id, r.dry, r.journal.as_deref(), r.log.as_deref())?;
            Ok(serde_json::json!({"ok": true, "exit": o.exit, "cost": o.cost, "instret": o.instret, "secs": t.elapsed().as_secs_f64()}))
        }
        "prove" => {
            let input = r.input.as_ref().ok_or_else(|| eyre::eyre!("prove: input"))?;
            let out = r.out.as_ref().ok_or_else(|| eyre::eyre!("prove: out"))?;
            crate::r0::prove_with_known(e, dir, input, &r.children, r.self_id, r.journal.as_deref(), out)?;
            Ok(serde_json::json!({"ok": true, "secs": t.elapsed().as_secs_f64()}))
        }
        "prove_segments" => {
            // segpar: segments lo..hi of the program on a serialized stdin (input), their proofs to out
            let input = r.input.as_ref().ok_or_else(|| eyre::eyre!("prove_segments: input"))?;
            let out = r.out.as_ref().ok_or_else(|| eyre::eyre!("prove_segments: out"))?;
            let n = crate::segpar::prove_segments(e, dir, input, r.lo, r.hi, out)?;
            Ok(serde_json::json!({"ok": true, "segments": n, "secs": t.elapsed().as_secs_f64()}))
        }
        "image" => {
            let img = Engine::image(dir)?;
            Ok(serde_json::json!({"ok": true, "image_id": hex::encode(crate::r0::image_id(&img))}))
        }
        x => eyre::bail!("unknown op {x}"),
    }
}
