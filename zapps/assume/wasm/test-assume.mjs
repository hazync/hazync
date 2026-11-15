// node test-assume.mjs: the assumevalid / assumeutxo checks in the assume zAPP's own WASM module (zapp-assume.js;
// docs/libzync/ASSUME_ZAPPS.md), on the real mainnet 1..200 proof (the core's swiftsync-verify/vectors) and Core
// v31.1.0's dumptxoutset files (../vectors). Exit 1 on
// any unexpected verdict. A proof at 200 is below minimum work: a passing check is "demo", never "ok".
import fs from 'node:fs';
import { loadAssume } from './zapp-assume.js';
const d = new URL('../../../swiftsync-verify/vectors/', import.meta.url).pathname;
const own = new URL('../vectors/', import.meta.url).pathname;
const v = await loadAssume(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_assume_wasm.wasm', import.meta.url)));
const rd = (f) => new Uint8Array(fs.readFileSync(f.startsWith('assume/') ? own + f.slice(7) : d + f));
const receipt = rd('mainnet_1_200_8888d59c.rcpt'), flipped = rd('mainnet_1_200_8888d59c_flipped.rcpt');
const headers = rd('assume/headers_0_200.bin'), snap = rd('assume/utxo_200.dat');
const TIP = '000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const B170 = '00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee';
const H3 = 'a51f51aea7cf42fc9f1ab491532de766675e983fde7166fa6273a6d6a6439d67';
const tampered = snap.slice(); tampered[51] ^= 1;
const cases = [
  ['assumeutxo_200', { receipt, headers, snapshot: snap, expect: `200:${TIP}` }, 'demo', (r) => r.assume.assumeutxo.hash_serialized_3 === H3 && r.assume.assumeutxo.coins === 204],
  ['assumevalid_tip', { receipt, assumevalid: 'tip' }, 'demo', (r) => r.assume.assumevalid.block === TIP],
  ['assumevalid_170', { receipt, assumevalid: B170, headers }, 'demo', (r) => r.assume.assumevalid.height === 170],
  ['tampered_snapshot', { receipt, headers, snapshot: tampered }, 'fail'],
  ['snapshot_199', { receipt, headers, snapshot: rd('assume/utxo_199.dat') }, 'fail'],
  ['snapshot_1003', { receipt, headers, snapshot: rd('assume/utxo_1003.dat') }, 'fail', (r) => r.assume.checks.some((c) => c.what.includes('not the proven tip'))],
  ['no_headers', { receipt, snapshot: snap }, 'fail'],
  ['wrong_expect', { receipt, headers, snapshot: snap, expect: `200:${B170}` }, 'fail'],
  ['flipped_receipt', { receipt: flipped, headers, snapshot: snap }, 'fail'],
  ['tracked_false', { receipt, headers, snapshot: snap, tracked: '0e3e2357e806b6cdb1f70b54c3a3a17b6714ee1f0e68bebb44a74b1efd512098:0' }, 'fail'],
  ['nothing_asked', { receipt }, 'fail'],
];
let bad = 0;
for (const [name, args, want, extra] of cases) {
  const r = v.verifyAssume({ network: 'mainnet', ...args });
  const ok = r.verdict === want && (!extra || extra(r));
  bad += !ok;
  const why = (r.assume?.checks || []).concat(r.checks).filter((c) => !c.ok).map((c) => c.what)[0] || '';
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(18)} ${r.verdict} (want ${want})  ${why.slice(0, 140)}`);
}
console.log(`${cases.length - bad}/${cases.length} cases as expected (wasm, assume)`);
process.exit(bad ? 1 : 0);
