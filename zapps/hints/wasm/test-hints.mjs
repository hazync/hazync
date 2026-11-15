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
console.log(`HINTS_WASM ${cases.length - bad}/${cases.length} (image ${v.hintsImageId()})`);
process.exit(bad ? 1 : 0);
