// node test-liveness.mjs: liveness proofs (docs/libzync/LIVENESS_ZAPP.md) through the liveness zAPP's own WASM module
// (zapp-liveness.js), against the real proof of mainnet 1..200 (8888d59c, the core's swiftsync-verify/vectors). Honest
// claims give "demo" (below minimum work), refusals "fail". (The core verifier's facts frame has its own regression
// test, swiftsync-verify-wasm/test-facts-frame.mjs.)
import fs from 'node:fs';
import { loadLiveness } from './zapp-liveness.js';
const core = new URL('../../../swiftsync-verify/vectors/', import.meta.url).pathname;
const own = new URL('../vectors/', import.meta.url).pathname;
const v = await loadLiveness(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_liveness_wasm.wasm', import.meta.url)));
const rd = (f) => fs.readFileSync(core + f);
const hex = (f) => fs.readFileSync(own + f, 'utf8').trim();
const T9 = '0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9';
const T170 = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';
const TIP = '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const flip = (h, i) => h.slice(0, i) + (h[i] === '0' ? '1' : '0') + h.slice(i + 1);
const cases = [
  ['spent at 170', 'b9_0_at200.hex', `${T9}:0@200=spent:170:1:0`, 'demo'],
  ['unspent after 169', 'b9_0_at169.hex', `${T9}:0@169=unspent`, 'demo'],
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
  ['dev-mode receipt', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', 'fake_block_2501.rcpt'],
];
let bad = 0;
for (const [name, f, ask, want, rc, expect] of cases) {
  const proof = Array.isArray(f) ? (f[1] === 'nl' ? hex(f[0]) + '\n' : flip(hex(f[0]), f[1])) : hex(f);
  const r = v.verifyLiveness({ network: 'mainnet', image: 'product', imageHex: '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4',
    receipt: rd(rc || 'mainnet_1_200_8888d59c.rcpt'), liveness: [{ proof, ask }], expect });
  const ok = r.verdict === want && (want !== 'demo' || ((r.checks || []).every((c) => c.ok) && r.facts.length === 1));
  bad += !ok;
  const why = (r.facts || []).map((x) => x.what).concat((r.checks || []).filter((c) => !c.ok).map((c) => c.what))[0] || '';
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})  ${why.slice(0, 110)}`);
}
// review 11-16: an image id given is the only one the receipt may verify against (the current mainnet id given refuses
// this 8888d59c receipt); none given, the earlier built-in image the receipt names verifies it and is reported
for (const [name, imageHex, want, img] of [['image pinned: current id', '46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c', 'fail'],
                                            ['no image id: reported', '', 'demo', '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4']]) {
  const r = v.verifyLiveness({ network: 'mainnet', image: 'product', imageHex, receipt: rd('mainnet_1_200_8888d59c.rcpt'),
    liveness: [{ proof: hex('b170_0_at200.hex'), ask: `${T170}:0@200=unspent` }] });
  const ok = r.verdict === want && (!img || r.image === img);
  bad += !ok; cases.push(name);
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want}) image ${r.image}`);
}
// positive control: the shipped module refuses the zAPP regtest harness's real receipt (image eb2cd424) with its real
// liveness proof (X spent at a 111, scripts/zapp-regtest): mainnet and signet images do not verify it, "regtest" is no
// network here
{
  const rt = fs.readFileSync(new URL('../../../scripts/zapp-regtest/receipts/a/block_112.rcpt', import.meta.url));
  const ask = 'e0b6ab75f1ce5f1dcf1a77ded4bf03eb962f50365ee8f77a4b269802dadd5a9f:1@112=spent:111:2:0';
  for (const [network, image] of [['mainnet', 'product'], ['signet', 'product'], ['regtest', 'product']]) {
    const r = v.verifyLiveness({ network, image, receipt: rt, liveness: [{ proof: hex('regtest_x_spent_a112.hex'), ask }],
      expect: '112:67c1d2e718067bba4ffd07a10f2ce27f5e6b55702eb2c766fafbf36db6160574' });
    const ok = r.verdict === 'fail';
    bad += !ok; cases.push(`regtest receipt, ${network}`);
    console.log(`${ok ? 'PASS' : 'WRONG'}  ${('regtest rcpt ' + network).padEnd(22)} ${r.verdict} (want fail)`);
  }
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
const parts = [rd('mainnet_1_200_8888d59c.rcpt'), enc.encode(''), enc.encode('8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4'),
  enc.encode(JSON.stringify({ liveness: [{ proof: hex('b170_0_at200.hex'), ask: `${T170}:0@200=unspent` }] })), enc.encode(TIP)];
const fr = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
fr[0] = 0x20;
const dv = new DataView(fr.buffer);
dv.setBigUint64(1, BigInt(Math.floor(Date.now() / 1000)), true);
let o = 9;
for (const p of parts) { dv.setUint32(o, p.length, true); fr.set(p, o + 4); o += 4 + p.length; }
const trailing = new Uint8Array(fr.length + 1); trailing.set(fr);
for (const [name, bytes, want, why] of [['frame: the glue\'s bytes', fr, 'demo', ''], ['frame: a byte past the end', trailing, 'fail', 'trailing'],
                                        ['frame: cut short', fr.slice(0, fr.length - 1), 'fail', 'truncated']]) {
  const r = raw(bytes);
  const ok = r.verdict === want && (!why || JSON.stringify(r).includes(why));
  bad += !ok;
  cases.push([name]);
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})`);
}
console.log(`LIVENESS_WASM ${cases.length - bad}/${cases.length} cases as expected`);
process.exit(bad ? 1 : 0);
