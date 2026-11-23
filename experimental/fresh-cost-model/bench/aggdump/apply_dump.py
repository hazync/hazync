#!/usr/bin/env python3
"""Measurement-only hook for Hazync's host: record the EXACT input stream of the final aggregate execution in
agg-exec-fake (every b.write / b.write_slice of write_aggregate_env_ex + libzync_agg_tail, as RISC Zero's serde words /
raw slice bytes, in order) and write it to $HAZYNC_AGG_INPUT_DUMP. Changes nothing the guest sees."""
import sys, re
p = sys.argv[1] + "/prover/host/src/lz.rs"; s = open(p).read()
REC = '''
/// MEASUREMENT (fresh-cost-model): the final aggregate's input stream, recorded when HAZYNC_AGG_INPUT_DUMP is set.
pub(crate) static AGG_REC: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);
pub(crate) fn rec_w<T: serde::Serialize + ?Sized>(x: &T) {
    if let Some(v) = AGG_REC.lock().unwrap().as_mut() { for w in risc0_zkvm::serde::to_vec(x).unwrap() { v.extend_from_slice(&w.to_le_bytes()); } }
}
/// Raw bytes of a slice of plain words (u8 / u32), exactly what write_slice sends.
pub(crate) fn rec_s<T: Copy>(x: &[T]) {
    if let Some(v) = AGG_REC.lock().unwrap().as_mut() {
        v.extend_from_slice(unsafe { core::slice::from_raw_parts(x.as_ptr() as *const u8, core::mem::size_of_val(x)) });
    }
}
'''
def fn_span(name):
    i = s.index(f"pub(crate) fn {name}(")
    j = s.index("\n}\n", i) + 3
    return i, j
def wrap(body, call, rec):
    out, k = [], 0
    while True:
        i = body.find(call, k)
        if i < 0: out.append(body[k:]); return "".join(out)
        j = i + len(call); depth = 1
        while depth:
            depth += {"(": 1, ")": -1}.get(body[j], 0); j += 1
        arg = body[i + len(call):j - 1]
        out.append(body[k:i]); out.append("{ let __x = " + arg + "; " + rec + "(__x); " + call + "__x) }"); k = j
for name in ["write_aggregate_env_ex", "libzync_agg_tail"]:
    i, j = fn_span(name)
    body = wrap(wrap(s[i:j], "b.write(", "rec_w"), "b.write_slice(", "rec_s")
    s = s[:i] + body + s[j:]
s = s.replace("pub(crate) fn report_agg_execute_only(", REC + "\npub(crate) fn report_agg_execute_only(", 1)
# dump once execution is set up: at the top of report_agg_execute_only (the env is complete by then)
s = s.replace("""    use std::time::Instant;
    let t_x = Instant::now();
    let session = default_executor().execute(b.build().unwrap(), METHOD_ELF)""", """    use std::time::Instant;
    if let (Ok(p), Some(v)) = (std::env::var("HAZYNC_AGG_INPUT_DUMP"), AGG_REC.lock().unwrap().take()) {
        std::fs::write(&p, &v).expect("HAZYNC_AGG_INPUT_DUMP"); println!("  aggregate input recorded: {} bytes -> {p}", v.len());
    }
    let t_x = Instant::now();
    let session = default_executor().execute(b.build().unwrap(), METHOD_ELF)""", 1)
# start recording right before the FINAL aggregate env is built in agg_exec_fake (after cl_synth_around's collect run)
old = """    if methods::COINLOG { cl_synth_around(&receipts, &mut anchor, &mut w, false); }
    let mut b = ExecutorEnv::builder();
    write_aggregate_env(&mut b, &receipts, &anchor, &w);"""
assert s.count(old) == 1, "agg_exec_fake anchor"
s = s.replace(old, """    if methods::COINLOG { cl_synth_around(&receipts, &mut anchor, &mut w, false); }
    if std::env::var("HAZYNC_AGG_INPUT_DUMP").is_ok() { *AGG_REC.lock().unwrap() = Some(Vec::new()); }
    let mut b = ExecutorEnv::builder();
    write_aggregate_env(&mut b, &receipts, &anchor, &w);""")
open(p, "w").write(s)
print("dump hook inserted")
