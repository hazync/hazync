// node test-liveness.mjs: liveness proofs (docs/libzync/LIVENESS_ZAPP.md) through the liveness zAPP's own WASM module
// (zapp-liveness.js), against the engine chain's proof of mainnet 1..200 (swiftsync-verify/tests/data/engine200; the
// proofs in ../vectors/engine, node format 2). Honest claims give "demo" (below minimum work), refusals "fail". Engine
// proofs only since the cutover (27.11.2026). (The core verifier's facts frame has its own regression test,
// swiftsync-verify-wasm/test-facts-frame.mjs.)
import fs from 'node:fs';
import { loadLiveness } from './zapp-liveness.js';
const E = new URL('../../../swiftsync-verify/tests/data/engine200/', import.meta.url).pathname;
const own = new URL('../vectors/engine/', import.meta.url).pathname;
const v0 = await loadLiveness(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_liveness_wasm.wasm', import.meta.url)));
const hex = (f) => fs.readFileSync(own + f, 'utf8').trim();
const receipt = fs.readFileSync(E + 'proof.hze');
const imageHex = fs.readFileSync(E + 'image_id', 'utf8').trim(), vk = new Uint8Array(fs.readFileSync(E + 'vk.bin'));
const pin = new Uint8Array(Buffer.from(fs.readFileSync(E + 'vk.sha256', 'utf8').trim(), 'hex'));
const v = { verifyLiveness: (a) => v0.verifyLiveness({ imageHex, vk, pin, ...a }) };
const T9 = '0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9';
const T170 = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';
const TIP = '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const flip = (h, i) => h.slice(0, i) + (h[i] === '0' ? '1' : '0') + h.slice(i + 1);
const k2 = vk.slice(); k2[100] ^= 1;
const cases = [
  ['spent at 170', 'b9_0_at200.hex', `${T9}:0@200=spent:170:1:0`, 'demo'],
  ['unspent after 169', 'b9_0_at169.hex', `${T9}:0@169=unspent`, 'demo'],
  ['unspent after 170', 'b170_1_at170.hex', `${T170}:1@170=unspent`, 'demo'],
  ['unspent at the tip', 'b170_0_at200.hex', `${T170}:0@tip=unspent,value=1000000000`, 'demo'],
  ['spent at 181', 'b170_1_at200.hex', `${T170}:1@200=spent:181`, 'demo'],
  ['with the tip expected', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'demo', undefined, TIP],
  ['false claim: unspent', 'b9_0_at200.hex', `${T9}:0@200=unspent`, 'fail'],
  ['false claim: spent', 'b170_0_at200.hex', `${T170}:0@200=spent`, 'fail'],
  ['no claim', 'b170_0_at200.hex', `${T170}:0@200`, 'fail'],
  ['wrong amount', 'b170_0_at200.hex', `${T170}:0@200=unspent,value=1`, 'fail'],
  ['another height', 'b170_0_at200.hex', `${T170}:0@150=unspent`, 'fail'],
  ['tampered proof', ['b170_0_at200.hex', 300], `${T170}:0@200=unspent`, 'fail'],
  ['another tip expected', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', undefined, `200:${'0'.repeat(63)}1`],
  ['created pinned', 'b170_0_at200.hex', `${T170}:0@200=unspent,created=170,coinbase=0`, 'demo'],
  ['wrong creation height', 'b170_0_at200.hex', `${T170}:0@200=unspent,created=171`, 'fail'],
  ['proof hex with a newline', ['b170_0_at200.hex', 'nl'], `${T170}:0@200=unspent`, 'fail'],
  ['another image id', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', { imageHex: 'ab'.repeat(32) }],
  ['a key not the pinned one', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', { vk: k2 }],
  ['no key', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', { vk: undefined, pin: undefined }],
];
let bad = 0;
for (const [name, f, ask, want, over, expect] of cases) {
  const proof = Array.isArray(f) ? (f[1] === 'nl' ? hex(f[0]) + '\n' : flip(hex(f[0]), f[1])) : hex(f);
  const r = v.verifyLiveness({ network: 'mainnet', image: 'product', receipt, liveness: [{ proof, ask }], expect, ...(over || {}) });
  const ok = r.verdict === want && (want !== 'demo' || ((r.checks || []).every((c) => c.ok) && r.facts.length === 1));
  bad += !ok;
  const why = (r.facts || []).map((x) => x.what).concat((r.checks || []).filter((c) => !c.ok).map((c) => c.what))[0] || '';
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})  ${why.slice(0, 110)}`);
}
// The frame itself, by hand (the raw exports, not the glue): the glue's bytes pass, a byte past the end and a frame
// cut short are refused before anything is verified.
const ex = (await WebAssembly.instantiate(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_liveness_wasm.wasm', import.meta.url)), {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_liveness(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const frame = (factsJson) => {
  const parts = [receipt, enc.encode(''), enc.encode(imageHex), enc.encode(factsJson), enc.encode(TIP), vk, pin];
  const fr = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  fr[0] = 0x20;
  const dv = new DataView(fr.buffer);
  dv.setBigUint64(1, BigInt(Math.floor(Date.now() / 1000)), true);
  let o = 9;
  for (const p of parts) { dv.setUint32(o, p.length, true); fr.set(p, o + 4); o += 4 + p.length; }
  return fr;
};
const F = JSON.stringify({ proof: hex('b170_0_at200.hex'), ask: `${T170}:0@200=unspent` });
const fr = frame(`{"liveness": [${F}]}`);
const trailing = new Uint8Array(fr.length + 1); trailing.set(fr);
for (const [name, bytes, want, why] of [['frame: the glue\'s bytes', fr, 'demo', ''], ['frame: a byte past the end', trailing, 'fail', 'trailing'],
                                        ['frame: cut short', fr.slice(0, fr.length - 1), 'fail', 'truncated']]) {
  const r = raw(bytes);
  const ok = r.verdict === want && (!why || JSON.stringify(r).includes(why));
  bad += !ok;
  cases.push([name]);
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})`);
}
// Review 11-18: the facts JSON is {"liveness": [...]} and nothing else (zapp_liveness::facts_from_json). Before it, a
// key beside "liveness" was dropped unread: the misspelt and the twice-given key verified as "demo".
for (const [name, j, want, why] of [['facts JSON: documented key', `{"liveness": [${F}]}`, 'demo', ''],
                                    ['facts JSON: misspelt beside', `{"liveness": [${F}], "livenes": [${F}]}`, 'fail', 'unknown field `livenes`'],
                                    ['facts JSON: misspelt alone', `{"livenes": [${F}]}`, 'fail', 'unknown field `livenes`'],
                                    ['facts JSON: key twice', `{"liveness": [${F}], "liveness": [${F}]}`, 'fail', 'duplicate field `liveness`'],
                                    ['facts JSON: expect inside', `{"liveness": [${F}], "expect": "${TIP}"}`, 'fail', '"expect" inside'],
                                    ['facts JSON: expect null', `{"liveness": [${F}], "expect": null}`, 'fail', '"expect" inside'],
                                    ['facts JSON: an array', `[[${F}]]`, 'fail', 'not an object'],
                                    ['facts JSON: fact extra key', `{"liveness": [{"x": 1, "proof": "00", "ask": "q"}]}`, 'fail', 'unknown field `x`']]) {
  const r = raw(frame(j));
  const ok = r.verdict === want && (!why || JSON.stringify(r).includes(JSON.stringify(why).slice(1, -1)));
  bad += !ok;
  cases.push([name]);
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})  ${JSON.stringify(r.checks || r).slice(0, 110)}`);
}
console.log(`LIVENESS_WASM ${cases.length - bad}/${cases.length} cases as expected`);
process.exit(bad ? 1 : 0);
