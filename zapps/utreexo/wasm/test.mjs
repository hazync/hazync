// node test.mjs: the real receipts of ../vectors through the WASM (the same cases as the library's real_receipts test).
import { readFileSync } from 'node:fs';
import { loadUtreexo } from './zapp-utreexo.js';
const H3 = '0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449';
const H2 = '000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd';
const u = await loadUtreexo(readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm', import.meta.url)));
const rd = (f) => new Uint8Array(readFileSync(new URL(`../vectors/${f}`, import.meta.url)));
let pass = 0, fail = 0;
const t = (name, r, want) => { if (r.verdict === want) pass++; else { fail++; console.log('FAIL', name, r.verdict, JSON.stringify(r.checks.filter(c => !c.ok))); } };
t('H=3 demo', u.verify({ receipt: rd('mainnet-3.receipt'), expectAt: `3:${H3}` }), 'demo');
t('H=3 wrong hash', u.verify({ receipt: rd('mainnet-3.receipt'), expectAt: `3:${H2}` }), 'fail');
t('H=3 wrong height', u.verify({ receipt: rd('mainnet-3.receipt'), expectAt: `2:${H3}` }), 'fail');
t('H=3 other image', u.verify({ receipt: rd('mainnet-3.receipt'), imageHex: 'ab'.repeat(32), expectAt: `3:${H3}` }), 'fail');
t('H=2 under T=3', u.verify({ receipt: rd('mainnet-2.receipt'), expectAt: `2:${H2}`, expectTip: `3:${H3}` }), 'demo');
t('H=3 tip given, T = H, wrong', u.verify({ receipt: rd('mainnet-3.receipt'), expectAt: `3:${H3}`, expectTip: `3:${H2}` }), 'fail');
t('H=3 two runs', u.verify({ receipt: rd('mainnet-3-split.receipt'), expectAt: `3:${H3}` }), 'demo');
console.log(`ZAPP_UTREEXO_WASM ${pass} passed, ${fail} failed; image ${u.imageId()}`);
process.exit(fail ? 1 : 0);
