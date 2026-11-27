// node test-hints.mjs <tests/data/engine> <hash of block 200> <core image id>: the WASM hints verifier on the engine's
// real hints proof for H = 200 (over the engine chain's core proof of mainnet 1..200): the proven file is DEMO (a
// 200-block chain is below minimum work), tampered inputs FAIL. Engine proofs only since the cutover (27.11.2026).
import fs from 'node:fs';
import { loadHints } from './zapp-hints.js';
const [d, h, core] = process.argv.slice(2);
const v = await loadHints(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_hints_wasm.wasm', import.meta.url)));
const rd = (f) => new Uint8Array(fs.readFileSync(`${d}/${f}`));
const receipt = rd('proof.hze'), file = rd('mainnet-200.hints'), undo = rd('mainnet-200.undo');
const imageHex = fs.readFileSync(`${d}/image_id`, 'utf8').trim(), vk = rd('vk.bin');
const pin = new Uint8Array(Buffer.from(fs.readFileSync(`${d}/vk.sha256`, 'utf8').trim(), 'hex'));
const id = { imageHex, coreImageHex: core, vk, pin };
const flip = file.slice(); flip[flip.length - 1] ^= 1;
const cases = [
  ['the proven file', { ...id, receipt, file, undo, expect: `200:${h}` }, 'demo'],
  ['a flipped bit', { ...id, receipt, file: flip, expect: `200:${h}` }, 'fail'],
  ['another hash at H', { ...id, receipt, file, expect: `200:${'0'.repeat(63)}1` }, 'fail'],
  ['a 9-byte file claiming H = 2^32 - 1', { ...id, receipt, file: new Uint8Array([0x55, 0x54, 0x58, 0x4f, 0, 0xff, 0xff, 0xff, 0xff]), expect: `200:${h}` }, 'fail'],
  ['a proof of another image', { ...id, receipt, file, imageHex: 'ab'.repeat(32) }, 'fail'],
  ['a key that is not the pinned one', { ...id, receipt, file, vk: (() => { const k = vk.slice(); k[100] ^= 1; return k; })() }, 'fail'],
  ['no key (no engine image is published yet)', { receipt, file, imageHex, coreImageHex: core }, 'fail'],
];
let bad = 0;
for (const [n, a, want] of cases) {
  const r = v.verifyHints(a);
  bad += r.verdict !== want;
  console.log(`${r.verdict === want ? 'PASS' : 'WRONG'} ${n}: ${r.verdict}${r.verdict === 'fail' ? ' (' + r.checks.find((c) => !c.ok).what.slice(0, 90) + ')' : ' height ' + r.height + ' unspent ' + r.unspent}`);
}
// The frame itself, by hand (the raw exports, not the glue): the proven call's bytes pass, a byte past the end and a
// frame cut short are refused before anything is verified.
const ex = (await WebAssembly.instantiate(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_hints_wasm.wasm', import.meta.url)), {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_hints(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const parts = [receipt, file, undo, enc.encode(imageHex), enc.encode(core), enc.encode(`200:${h}`), new Uint8Array(0), vk, pin];
const frame = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
new DataView(frame.buffer).setBigUint64(0, BigInt(Math.floor(Date.now() / 1000)), true);
let o = 8;
for (const p of parts) { new DataView(frame.buffer).setUint32(o, p.length, true); frame.set(p, o + 4); o += 4 + p.length; }
const trailing = new Uint8Array(frame.length + 1); trailing.set(frame);
for (const [n, bytes, want, why] of [['the frame as the glue writes it', frame, 'demo', ''],
                                     ['a byte past the end of the frame', trailing, 'fail', 'trailing'],
                                     ['a frame cut short', frame.slice(0, frame.length - 1), 'fail', 'truncated']]) {
  const r = raw(bytes);
  const ok = r.verdict === want && (!why || r.checks.some((c) => !c.ok && c.what.includes(why)));
  bad += !ok;
  cases.push([n]);
  console.log(`${ok ? 'PASS' : 'WRONG'} ${n}: ${r.verdict}${why ? ' (' + (r.checks.find((c) => !c.ok) || {}).what + ')' : ''}`);
}
console.log(`HINTS_WASM ${cases.length - bad}/${cases.length}`);
process.exit(bad ? 1 : 0);
