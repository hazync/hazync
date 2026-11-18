// node test-hints.mjs <dir with mainnet-3.{receipt,hints,undo} and mainnet-2.hints> <hash of block 3>: the WASM hints
// verifier on a real receipt: the proven file is DEMO (a 3-block chain is below minimum work), tampered inputs FAIL.
import fs from 'node:fs';
import { loadHints } from './zapp-hints.js';
const [d, h3] = process.argv.slice(2);
const v = await loadHints(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_hints_wasm.wasm', import.meta.url)));
const rd = (f) => new Uint8Array(fs.readFileSync(`${d}/${f}`));
const receipt = rd('mainnet-3.receipt'), file = rd('mainnet-3.hints'), undo = rd('mainnet-3.undo');
const flip = file.slice(); flip[flip.length - 1] ^= 1;
const cases = [
  ['the proven file', { receipt, file, undo, expect: `3:${h3}` }, 'demo'],
  ['a flipped bit', { receipt, file: flip, expect: `3:${h3}` }, 'fail'],
  ['the file for H = 2', { receipt, file: rd('mainnet-2.hints'), expect: `3:${h3}` }, 'fail'],
  ['another hash at H', { receipt, file, expect: `3:${'0'.repeat(63)}1` }, 'fail'],
  ['a 9-byte file claiming H = 2^32 - 1', { receipt, file: new Uint8Array([0x55, 0x54, 0x58, 0x4f, 0, 0xff, 0xff, 0xff, 0xff]), expect: `3:${h3}` }, 'fail'],
  ['a receipt of another image', { receipt, file, imageHex: 'ab'.repeat(32) }, 'fail'],
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
const parts = [receipt, file, undo, new Uint8Array(0), new Uint8Array(0), enc.encode(`3:${h3}`), new Uint8Array(0)];
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
console.log(`HINTS_WASM ${cases.length - bad}/${cases.length} (image ${v.hintsImageId()})`);
process.exit(bad ? 1 : 0);
