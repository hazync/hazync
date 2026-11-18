// node test-statement.mjs <zapp-statement.wasm> <statement.json> <purpose> <nonce>: the balance
// statement through the zAPP's WASM module, against the real mainnet 1..200 proof (8888d59c), with the right and wrong challenge.
// The statement comes from ../test_statement.sh's flow (zync-statement draft + sign). Exit 1 on any
// unexpected verdict. 1..200 is below the minimum work, so the best verdict is demo.
import fs from 'node:fs';
import { loadStatementVerifier } from './zapp-statement.js';
const [wasm, stFile, purpose, nonce] = process.argv.slice(2);
const v = await loadStatementVerifier(fs.readFileSync(wasm));
const dir = new URL('../../../swiftsync-verify/vectors/', import.meta.url).pathname;
const receipt = fs.readFileSync(dir + 'mainnet_1_200_8888d59c.rcpt');
const statement = JSON.parse(fs.readFileSync(stFile, 'utf8'));
const expect = '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const base = { network: 'mainnet', imageHex: '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4', receipt, expect };
const cases = [
  ['holds (demo: below minimum work)', { statement, purpose, nonce }, 'demo', /^statement: .* script\(s\).*age is NOT checked/],
  ['another purpose', { statement, purpose: 'other', nonce }, 'fail', /purpose .* is not the one asked for/],
  ['another nonce', { statement, purpose, nonce: 'other' }, 'fail', /nonce .* is not the one asked for/],
  ['tampered total', { statement: { ...statement, total: statement.total + 1 }, purpose, nonce }, 'fail', /facts sum to/],
  ['wrong height', { statement: { ...statement, height: 199 }, purpose, nonce }, 'fail', /about block 199/],
  ['another tip expected', { statement, purpose, nonce, expect: '200:' + '0'.repeat(64) }, 'fail', /not the proof's tip/],
  ['age checked (6 behind, 6 allowed)', { statement, purpose, nonce, maxAge: 6, tipHeight: 206 }, 'demo', /6 block\(s\) behind the network's tip 206/],
  ['too old (7 behind, 6 allowed)', { statement, purpose, nonce, maxAge: 6, tipHeight: 207 }, 'fail', /7 blocks behind/],
  ['max age without a tip', { statement, purpose, nonce, maxAge: 6 }, 'fail', /without the network's tip/],
  ['a purpose with a trailing space', { statement: { ...statement, purpose: statement.purpose + ' ' }, purpose: purpose + ' ', nonce }, 'fail', /leading or trailing/],
  // review 11-16: an image id given is the only image the receipt may verify against (the current mainnet id refuses
  // this 8888d59c receipt); none given, the earlier built-in image it names verifies it
  ['image pinned: the current id', { statement, purpose, nonce, imageHex: '46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c' }, 'fail', /the receipt did not verify/],
  ['no image id: the image it names', { statement, purpose, nonce, imageHex: '' }, 'demo', /^statement: /],
  ['swapped signatures', { statement: { ...statement, entries: statement.entries.map((e, i, a) => ({ ...e, signature: a[(i + 1) % a.length].signature })) }, purpose, nonce }, 'fail', /BIP-322|P2PK/],
];
let bad = 0;
for (const [name, args, want, why] of cases) {
  const r = v.verifyStatement({ ...base, ...args });
  const ok = r.verdict === want && why.test(r.statement?.what ?? '');
  bad += !ok;
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(34)} ${r.verdict} (want ${want}): ${r.statement?.what ?? r.checks?.[0]?.what}`);
}
const fake = v.verifyStatement({ ...base, receipt: fs.readFileSync(dir + 'fake_block_2501.rcpt'), statement, purpose, nonce });
bad += fake.verdict !== 'fail';
console.log(`${fake.verdict === 'fail' ? 'PASS' : 'WRONG'}  ${'dev-mode FakeReceipt'.padEnd(34)} ${fake.verdict} (want fail)`);
// review 11-18: the result names the image that verified the receipt (CoreCheck.image), not the one asked for: no image
// id given, this receipt verifies as the earlier built-in 8888d59c, and the JSON says so
const used = v.verifyStatement({ ...base, statement, purpose, nonce, imageHex: '' });
const usedOk = used.image === base.imageHex;
bad += !usedOk;
console.log(`${usedOk ? 'PASS' : 'WRONG'}  ${'no image id: reports 8888d59c'.padEnd(34)} ${used.image}`);
// The frame itself, by hand (the raw exports, not the glue): the glue's bytes pass, a byte past the end and a frame
// cut short are refused before anything is verified.
const ex = (await WebAssembly.instantiate(fs.readFileSync(wasm), {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_statement(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const parts = [receipt, enc.encode(base.imageHex), enc.encode(JSON.stringify(statement)), enc.encode(expect), enc.encode(purpose),
  enc.encode(nonce), enc.encode(''), enc.encode('')];
const fr = new Uint8Array(1 + 8 + parts.reduce((n, p) => n + 4 + p.length, 0));
const dv = new DataView(fr.buffer);
dv.setBigUint64(1, BigInt(Math.floor(Date.now() / 1000)), true);
let o = 9;
for (const p of parts) { dv.setUint32(o, p.length, true); fr.set(p, o + 4); o += 4 + p.length; }
const trailing = new Uint8Array(fr.length + 1); trailing.set(fr);
let nraw = 0;
for (const [name, bytes, want, why] of [['frame: the glue\'s bytes', fr, 'demo', ''], ['frame: a byte past the end', trailing, 'fail', 'trailing'],
                                        ['frame: cut short', fr.slice(0, fr.length - 1), 'fail', 'truncated']]) {
  const r = raw(bytes);
  const ok = r.verdict === want && (!why || JSON.stringify(r).includes(why));
  bad += !ok; nraw++;
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(34)} ${r.verdict} (want ${want})`);
}
console.log(`${cases.length + 2 + nraw - bad}/${cases.length + 2 + nraw} as expected (zapp-statement.wasm)`);
process.exit(bad ? 1 : 0);
