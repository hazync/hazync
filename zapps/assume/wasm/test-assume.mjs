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
  // review 11-16: an image id given is the only one the receipt may verify against; none given, the image that
  // verified it (the earlier built-in 8888d59c the receipt names) is the one reported
  ['image_pinned_current', { receipt, assumevalid: 'tip', imageHex: '46666b6f7e3d1c1c3fa599a9bcf12209e0730d6a2fb747b8bf30fac667e43c1c' }, 'fail'],
  ['image_reported', { receipt, assumevalid: 'tip' }, 'demo', (r) => r.image === '8888d59c4b8c699fd4cc3afaf8168ff2baa1f6511b1dcea1c23f041b2b2315f4'],
];
let bad = 0;
for (const [name, args, want, extra] of cases) {
  const r = v.verifyAssume({ network: 'mainnet', ...args });
  const ok = r.verdict === want && (!extra || extra(r));
  bad += !ok;
  const why = (r.assume?.checks || []).concat(r.checks).filter((c) => !c.ok).map((c) => c.what)[0] || '';
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(18)} ${r.verdict} (want ${want})  ${why.slice(0, 140)}`);
}
// The frame itself, by hand (the raw exports, not the glue): the glue's bytes pass, a byte past the end and a frame
// cut short are refused before anything is verified.
const ex = (await WebAssembly.instantiate(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_assume_wasm.wasm', import.meta.url)), {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_assume(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const parts = [receipt, enc.encode(''), enc.encode(`200:${TIP}`), enc.encode('tip'), headers, new Uint8Array(0), enc.encode('')];
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
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${name.padEnd(18)} ${r.verdict} (want ${want})`);
}
console.log(`${cases.length + nraw - bad}/${cases.length + nraw} cases as expected (wasm, assume)`);
process.exit(bad ? 1 : 0);
