// node test.mjs [<tests/data/engine>]: the utreexo zAPP's engine proofs (../tests/data/engine: the base, roots after block
// 100, and a tip step 100 -> 150, both on the engine chain's core proof of mainnet 1..200) through the WASM, the same
// cases as ../test.sh's CLI. Engine proofs only since the cutover (27.11.2026); no engine image is published yet, so
// every call gives the image ids, the key and its pin.
import { readFileSync } from 'node:fs';
import { loadUtreexo } from './zapp-utreexo.js';
const dir = process.argv[2] ?? new URL('../tests/data/engine', import.meta.url).pathname;
const W = new URL('./target/wasm32-unknown-unknown/release/zapp_utreexo_wasm.wasm', import.meta.url);
const u = await loadUtreexo(readFileSync(W));
const rd = (f) => new Uint8Array(readFileSync(`${dir}/${f}`));
const ids = Object.fromEntries(readFileSync(`${dir}/ids.txt`, 'utf8').trim().split('\n').filter((l) => l.includes(' ')).map((l) => l.split(' ')));
const [, , , PIN_BASE, PIN_TIP] = readFileSync(`${dir}/ids.txt`, 'utf8').trim().split('\n');
const hexb = (h) => new Uint8Array(Buffer.from(h, 'hex'));
const BASE = ids['keys-z-utreexo'], TIPI = ids['keys-z-utreexo-tip'], CORE = ids['keys-ch'];
const H100 = '000000007bc154e0fa7ea32218a72fe2c1bb9f86cf8c9ebf9a715ed27fdb229a';
const H150 = '000000009ca75733b4cf527fe193b919201a2ed38c9e147a5665fdfade551f4d';
const H200 = '000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const AB = 'ab'.repeat(32);
const P = rd('base100.hze'), R = rd('step150.hze');
const B = { receipt: P, imageHex: BASE, coreImageHex: CORE, vk: rd('base100.vk'), pin: hexb(PIN_BASE) };
const TK = { receipt: R, imageHex: TIPI, coreImageHex: CORE, vk: rd('step150.vk'), pin: hexb(PIN_TIP), baseImageHex: BASE };
let pass = 0, fail = 0;
const t = (name, r, want) => { if (r.verdict === want) pass++; else { fail++; console.log('FAIL', name, r.verdict, JSON.stringify(r.checks.filter(c => !c.ok)).slice(0, 300)); } };
t('base: block 100 and the core tip named', u.verify({ ...B, expectAt: `100:${H100}`, expectTip: `200:${H200}` }), 'demo');
t('base: another block 100', u.verify({ ...B, expectAt: `100:${H150}`, expectTip: `200:${H200}` }), 'fail');
t('base: another core tip', u.verify({ ...B, expectAt: `100:${H100}`, expectTip: `200:${H150}` }), 'fail');
t('base: T = 200 given as block 100\'s hash', u.verify({ ...B, expectAt: `100:${H100}`, expectTip: `100:${H100}` }), 'fail');
t('base: another image id', u.verify({ ...B, imageHex: AB, expectAt: `100:${H100}` }), 'fail');
t('base: another core image', u.verify({ ...B, coreImageHex: AB, expectAt: `100:${H100}` }), 'fail');
t('base: the tip\'s key', u.verify({ ...B, vk: TK.vk, pin: TK.pin, expectAt: `100:${H100}` }), 'fail');
t('base: another pin', u.verify({ ...B, pin: TK.pin, expectAt: `100:${H100}` }), 'fail');
t('base: no key (no engine image is published yet)', u.verify({ ...B, vk: undefined, pin: undefined, expectAt: `100:${H100}` }), 'fail');
t('base: checked as signet', u.verify({ ...B, network: 'signet', expectAt: `100:${H100}` }), 'fail');
t('base: an unknown network', u.verify({ ...B, network: 'regtest', expectAt: `100:${H100}` }), 'fail');
t('base: a base image id on a base proof', u.verify({ ...B, baseImageHex: AB, expectAt: `100:${H100}` }), 'fail');
t('base: a trailing byte', u.verify({ ...B, receipt: new Uint8Array([...P, 0]), expectAt: `100:${H100}` }), 'fail');
{
  const r = u.verify({ ...B, expectAt: `100:${H100}` });
  if (r.network === 'mainnet' && r.image_source === 'caller') pass++;
  else { fail++; console.log('FAIL network / image_source fields', r.network, r.image_source, r.image_label); }
}
{
  const r = u.verify({ ...TK, expectAt: `150:${H150}`, expectTip: `200:${H200}` });
  t('tip 150 demo', r, 'demo');
  if (r.mode === 'tip' && r.steps === 1 && r.from === 100 && r.base_height === 100 && r.height === 150 && r.core_height === 200) pass++;
  else { fail++; console.log('FAIL tip fields', JSON.stringify(r).slice(0, 400)); }
}
t('tip 150: wrong block', u.verify({ ...TK, expectAt: `150:${H100}` }), 'fail');
t('tip 150: wrong core tip', u.verify({ ...TK, expectAt: `150:${H150}`, expectTip: `200:${H150}` }), 'fail');
t('tip 150: another tip image', u.verify({ ...TK, imageHex: AB, expectAt: `150:${H150}` }), 'fail');
t('tip 150: another base image', u.verify({ ...TK, baseImageHex: AB, expectAt: `150:${H150}` }), 'fail');
t('tip 150: no base image', u.verify({ ...TK, baseImageHex: undefined, expectAt: `150:${H150}` }), 'fail');
t('tip 150: checked as signet', u.verify({ ...TK, network: 'signet', expectAt: `150:${H150}` }), 'fail');
{
  // the frame before the key (no core image, key or pin): refused as truncated, not read as a published image
  const { alloc, verify_utreexo, memory } = (await WebAssembly.instantiate(readFileSync(W), {})).instance.exports;
  const enc = new TextEncoder();
  const parts = [P, enc.encode(BASE), enc.encode(`100:${H100}`), enc.encode(''), enc.encode('mainnet')];
  const bytes = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const view = new DataView(bytes.buffer);
  view.setBigUint64(0, BigInt(Math.floor(Date.now() / 1000)), true);
  let o = 8;
  for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
  const ptr = alloc(bytes.length);
  new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
  const p = verify_utreexo(ptr, bytes.length);
  const len = new DataView(memory.buffer).getUint32(p, true);
  t('the frame without the key', JSON.parse(new TextDecoder().decode(new Uint8Array(memory.buffer, p + 4, len))), 'fail');
}
console.log(`ZAPP_UTREEXO_WASM ${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
