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
t('H=3 named mainnet', u.verify({ receipt: rd('mainnet-3.receipt'), network: 'mainnet', expectAt: `3:${H3}` }), 'demo');
t('H=3 checked as signet', u.verify({ receipt: rd('mainnet-3.receipt'), network: 'signet', expectAt: `3:${H3}` }), 'fail');
t('H=3 an unknown network', u.verify({ receipt: rd('mainnet-3.receipt'), network: 'regtest', expectAt: `3:${H3}` }), 'fail');
{
  const r = u.verify({ receipt: new Uint8Array([...rd('mainnet-3.receipt'), 0]), expectAt: `3:${H3}` });
  t('H=3 a receipt with a trailing byte', r, 'fail');
  if (!r.checks.some(c => !c.ok && c.what.startsWith('the receipt decodes'))) { fail++; console.log('FAIL the trailing byte is not refused by the decode', JSON.stringify(r.checks)); }
}
{
  // the frame before 18.11.2026 (no network field): refused, not read as mainnet
  const wasm = await WebAssembly.instantiate(readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm', import.meta.url)), {});
  const { alloc, verify_utreexo, memory } = wasm.instance.exports;
  const enc = new TextEncoder();
  const parts = [rd('mainnet-3.receipt'), enc.encode(''), enc.encode(`3:${H3}`), enc.encode('')];
  const bytes = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const view = new DataView(bytes.buffer);
  view.setBigUint64(0, BigInt(Math.floor(Date.now() / 1000)), true);
  let o = 8;
  for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
  const ptr = alloc(bytes.length);
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  const p = verify_utreexo(ptr, bytes.length);
  const len = new DataView(memory.buffer).getUint32(p, true);
  const r = JSON.parse(new TextDecoder().decode(new Uint8Array(memory.buffer, p + 4, len)));
  t('the old frame without a network field', r, 'fail');
}
{
  const r = u.verify({ receipt: rd('mainnet-3.receipt'), expectAt: `3:${H3}` });
  if (r.network === 'mainnet' && r.image_source === 'builtin-pinned' && r.image_label === null) pass++;
  else { fail++; console.log('FAIL network / image_source fields', r.network, r.image_source, r.image_label); }
}
// tip mode: the real step 3 -> 103 on the core proof 1..4,790 (vectors/core-spine-4790.receipt)
{
  const H103 = '000000002109057cb846ff7d1d96bc77f9f65cb547938efe1961760a8bbb3f75';
  const T = '0000000013371ed4d67e526a2fd2d8bd270b477c72bdacbd01048392f5528eb5';
  const R = rd('mainnet-tip-103.receipt');
  const r = u.verify({ receipt: R, expectAt: `103:${H103}`, expectTip: `4790:${T}` });
  t('tip 103 demo', r, 'demo');
  if (r.mode === 'tip' && r.steps === 1 && r.from === 3 && r.base_height === 3 && r.height === 103 && r.leaves === 103 && r.core_height === 4790) pass++;
  else { fail++; console.log('FAIL tip fields', JSON.stringify(r)); }
  t('tip 103 wrong block', u.verify({ receipt: R, expectAt: `103:${H3}` }), 'fail');
  t('tip 103 wrong core tip', u.verify({ receipt: R, expectAt: `103:${H103}`, expectTip: `4790:${H103}` }), 'fail');
  t('tip 103 other tip image', u.verify({ receipt: R, imageHex: 'ab'.repeat(32), expectAt: `103:${H103}` }), 'fail');
  t('tip 103 other base image', u.verify({ receipt: R, baseImageHex: 'ab'.repeat(32), expectAt: `103:${H103}` }), 'fail');
  t('tip 103 checked as signet', u.verify({ receipt: R, network: 'signet', expectAt: `103:${H103}` }), 'fail');
  t('a base image id on a base receipt', u.verify({ receipt: rd('mainnet-3.receipt'), baseImageHex: 'ab'.repeat(32), expectAt: `3:${H3}` }), 'fail');
  {
    // review H1: a composite receipt nested 300,000 levels deep: refused before decoding (no trap, a 'fail' JSON)
    const n = 300000, b = new Uint8Array(4 + 20 * n), v = new DataView(b.buffer);
    for (let i = 0; i < n; i++) v.setUint32(4 + 20 * i + 8, 1, true); // segments 0, assumption_receipts 1, tag 0
    const r = u.verify({ receipt: b, expectAt: `103:${H103}` });
    t('a composite receipt nested 300,000 deep', r, 'fail');
    if (!r.checks.some(c => !c.ok && c.what.includes('composite'))) { fail++; console.log('FAIL nested: not refused as composite', JSON.stringify(r.checks)); }
  }
  if (/^[0-9a-f]{64}$/.test(u.tipImageId())) pass++; else { fail++; console.log('FAIL tipImageId', u.tipImageId()); }
}
console.log(`ZAPP_UTREEXO_WASM ${pass} passed, ${fail} failed; image ${u.imageId()}`);
process.exit(fail ? 1 : 0);
