// node test-liveness.mjs: liveness proofs (docs/libzync/LIVENESS_ZAPP.md) through the WASM build's verifyFacts,
// against the real proof of mainnet 1..200 (8888d59c). Honest claims give "demo" (below minimum work), refusals "fail".
import fs from 'node:fs';
import { loadVerifier } from './swiftsync-verify.js';
const dir = new URL('../swiftsync-verify/vectors/', import.meta.url).pathname;
const v = await loadVerifier(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm', import.meta.url)));
const rd = (f) => fs.readFileSync(dir + f);
const hex = (f) => fs.readFileSync(dir + 'liveness/' + f, 'utf8').trim();
const T9 = '0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9';
const T170 = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';
const flip = (h, i) => h.slice(0, i) + (h[i] === '0' ? '1' : '0') + h.slice(i + 1);
const cases = [
  ['spent at 170', 'b9_0_at200.hex', `${T9}:0@200=spent:170:1:0`, 'demo'],
  ['unspent after 169', 'b9_0_at169.hex', `${T9}:0@169=unspent`, 'demo'],
  ['unspent at the tip', 'b170_0_at200.hex', `${T170}:0@tip=unspent,value=1000000000`, 'demo'],
  ['spent at 181', 'b170_1_at200.hex', `${T170}:1@200=spent:181`, 'demo'],
  ['false claim: unspent', 'b9_0_at200.hex', `${T9}:0@200=unspent`, 'fail'],
  ['false claim: spent', 'b170_0_at200.hex', `${T170}:0@200=spent`, 'fail'],
  ['no claim', 'b170_0_at200.hex', `${T170}:0@200`, 'fail'],
  ['wrong amount', 'b170_0_at200.hex', `${T170}:0@200=unspent,value=1`, 'fail'],
  ['another height', 'b170_0_at200.hex', `${T170}:0@150=unspent`, 'fail'],
  ['tampered proof', ['b170_0_at200.hex', 300], `${T170}:0@200=unspent`, 'fail'],
  ['dev-mode receipt', 'b170_0_at200.hex', `${T170}:0@200=unspent`, 'fail', 'fake_block_2501.rcpt'],
];
let bad = 0;
for (const [name, f, ask, want, rc] of cases) {
  const proof = Array.isArray(f) ? flip(hex(f[0]), f[1]) : hex(f);
  const r = v.verifyFacts({ network: 'mainnet', image: 'product', imageHex: '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4',
    receipt: rd(rc || 'mainnet_1_200_8888d59c.rcpt'), facts: { liveness: [{ proof, ask }] } });
  const ok = r.verdict === want;
  bad += !ok;
  const why = (r.facts || []).map((x) => x.what).concat((r.checks || []).filter((c) => !c.ok).map((c) => c.what))[0] || '';
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want ${want})  ${why.slice(0, 110)}`);
}
// the facts frame itself (regression, 15.11.2026: the optional lineage chunk swallowed the facts JSON, so every
// verifyFacts call with facts, and verify with an expected tip, failed): a real trie fact and an expect pass through
const trie = JSON.parse(fs.readFileSync(new URL('../experimental/libzync-bench/zapp/fact_balance_block9_200.json', import.meta.url)));
const base = { network: 'mainnet', image: 'product', imageHex: '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4', receipt: rd('mainnet_1_200_8888d59c.rcpt') };
for (const [name, r] of [['frame: a trie fact', v.verifyFacts({ ...base, facts: { tries: [trie] } })],
                         ['frame: verify + expect', v.verify({ ...base, expect: '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320' })]]) {
  const ok = r.verdict === 'demo' && (r.checks || []).every((c) => c.ok);
  bad += !ok;
  cases.push([name]);
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(22)} ${r.verdict} (want demo, every check ok)`);
}
console.log(`${cases.length - bad}/${cases.length} liveness cases as expected (wasm)`);
process.exit(bad ? 1 : 0);
