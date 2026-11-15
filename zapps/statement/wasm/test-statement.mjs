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
  ['holds (demo: below minimum work)', { statement, purpose, nonce }, 'demo', /^statement: .* script\(s\)/],
  ['another purpose', { statement, purpose: 'other', nonce }, 'fail', /purpose .* is not the one asked for/],
  ['another nonce', { statement, purpose, nonce: 'other' }, 'fail', /nonce .* is not the one asked for/],
  ['tampered total', { statement: { ...statement, total: statement.total + 1 }, purpose, nonce }, 'fail', /facts sum to/],
  ['wrong height', { statement: { ...statement, height: 199 }, purpose, nonce }, 'fail', /about block 199/],
  ['another tip expected', { statement, purpose, nonce, expect: '200:' + '0'.repeat(64) }, 'fail', /not the proof's tip/],
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
console.log(`${cases.length + 1 - bad}/${cases.length + 1} as expected (zapp-statement.wasm)`);
process.exit(bad ? 1 : 0);
