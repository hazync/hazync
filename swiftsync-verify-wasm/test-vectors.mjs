// node test-vectors.mjs [wasm]: run ../swiftsync-verify/vectors/manifest.json through the WASM build (raw frames,
// network bit 0x40: the case's --image and --vk, the key pinned by tests/data/engine200/vk.sha256) and compare each
// verdict with the case's expected exit code (0 ok, 2 demo, 1 fail). Exit 1 on any difference. Engine proofs only
// since the cutover (27.11.2026). Default wasm: the engine build's output.
import fs from 'node:fs';
import path from 'node:path';
const dir = new URL('../swiftsync-verify/vectors/', import.meta.url).pathname;
const wasm = process.argv[2] ?? new URL('./target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm', import.meta.url).pathname;
const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasm), {});
const x = instance.exports;
const pin = Buffer.from(fs.readFileSync(path.join(dir, '../tests/data/engine200/vk.sha256'), 'utf8').trim(), 'hex');
const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n); return b; };
const chunk = (b) => Buffer.concat([u32(b.length), b]);
function call(f) {
  const p = x.alloc(f.length); new Uint8Array(x.memory.buffer, p, f.length).set(f);
  const r = x.verify_frame(p, f.length); const n = new DataView(x.memory.buffer).getUint32(r, true);
  return JSON.parse(Buffer.from(new Uint8Array(x.memory.buffer, r + 4, n)).toString());
}
const m = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json')));
const code = { ok: 0, demo: 2, fail: 1 };
let bad = 0;
for (const c of m.cases.filter((c) => !c.cli_only)) {
  const a = { network: 'mainnet', files: [], now: Math.floor(Date.now() / 1000), image: '' };
  for (let i = 0; i < c.args.length; i++) {
    const v = c.args[i];
    if (v === '--network') a.network = c.args[++i];
    else if (v === '--image') a.image = c.args[++i];
    else if (v === '--vk') a.vk = fs.readFileSync(path.join(dir, c.args[++i]));
    else if (v === '--vk-sha256') a.pin = Buffer.from(c.args[++i], 'hex');   // the case's own pin (review R1-6 #1)
    else if (v === '--now') a.now = Number(c.args[++i]);
    else a.files.push(v);
  }
  const now = Buffer.alloc(8); now.writeBigUInt64LE(BigInt(a.now));
  const net = { mainnet: 0, signet: 1 }[a.network];
  const head = [Buffer.from([net | (a.vk ? 0x40 : 0)]), now, chunk(fs.readFileSync(path.join(dir, a.files[0]))),
                chunk(Buffer.alloc(0)), chunk(Buffer.from(a.image))];
  const r = call(Buffer.concat(a.vk ? [...head, chunk(a.vk), chunk(a.pin ?? pin)] : head));
  const ok = code[r.verdict] === c.expect_exit;
  bad += !ok;
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${c.name.padEnd(24)} ${r.verdict} (want exit ${c.expect_exit})`);
}
const n = m.cases.filter((c) => !c.cli_only).length;
console.log(`${n - bad}/${n} cases as expected (wasm)`);
process.exit(bad ? 1 : 0);
