// verifyDeposit in the deposit-proof zAPP's own WASM module (zapp-deposit.js), on bundles `zapp-deposit build` makes from
// the ENGINE proofs (as ../test_deposit.sh): block 170's Satoshi-to-Hal transaction against the engine chain's proof of
// mainnet 1..200, alone and bound to the engine's header chain 0..4,031. Both are below Core's minimum work, so the best
// real verdict is DEMO, never accepted (the unit tests reach OK on a zero-work image). Engine proofs only since the
// cutover (27.11.2026); no engine image is published yet, so every call gives the image ids, keys and pins.
// Usage: node test-deposit.mjs <zapp-deposit> [<zapp-deposit.js> <zapp_deposit_wasm.wasm>] (default: this directory's).
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const own = dirname(fileURLToPath(import.meta.url));
const [zd, js = `${own}/zapp-deposit.js`, wasm = `${own}/target/wasm32-unknown-unknown/release/zapp_deposit_wasm.wasm`] = process.argv.slice(2);
const root = resolve(own, '../../..');
const here = `${root}/experimental/libzync-bench/deposit`;
const E = `${root}/swiftsync-verify/tests/data/engine200`, H = `${root}/zapps/headers/tests/data/engine4031`;
const { loadDeposit } = await import(pathToFileURL(resolve(js)).href);
const v0 = await loadDeposit(readFileSync(wasm));
const txt = (f) => readFileSync(f, 'utf8').trim();
const pinOf = (f) => new Uint8Array(Buffer.from(txt(f), 'hex'));
const KEYS = { coreImageHex: txt(`${E}/image_id`), coreVk: new Uint8Array(readFileSync(`${E}/vk.bin`)), corePin: pinOf(`${E}/vk.sha256`),
  headersImageHex: txt(`${H}/image_id`), headersVk: new Uint8Array(readFileSync(`${H}/vk.bin`)), headersPin: pinOf(`${H}/vk.sha256`) };
const v = { verifyDeposit: (a) => v0.verifyDeposit({ ...KEYS, ...a }) };
const T = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';
const TIP = '4031:00000000f037ad09d0b05ee66b8c1da83030abaf909d2b1bf519c3c7d2cd3fdf';
const w = mkdtempSync(join(tmpdir(), 'deposit-wasm-'));
// the key pin the caller gives with every key (review R1-6 #1): here the fixture key's own SHA-256
const pin = (f) => createHash('sha256').update(readFileSync(f)).digest('hex');
const make = (out, extra) => execFileSync(resolve(zd), ['build', '--core', `${E}/proof.hze`, '--image', KEYS.coreImageHex, '--vk', `${E}/vk.bin`, '--vk-sha256', pin(`${E}/vk.bin`),
  '--history', `${E}/history.bin`, '--height', '170', '--block', `${here}/block_170_raw.txt`, '--txid', T, ...extra, '-o', out],
  { stdio: ['ignore', 'ignore', 'ignore'] });
make(`${w}/core.json`, []);
make(`${w}/tip.json`, ['--headers-image', KEYS.headersImageHex, '--headers-vk', `${H}/vk.bin`, '--headers-vk-sha256', pin(`${H}/vk.bin`), '--headers', `${H}/proof.hze`,
  '--tip-path', `${H}/mmr_path_200_tip4031.txt`]);
const core = readFileSync(`${w}/core.json`, 'utf8'), tip = JSON.parse(readFileSync(`${w}/tip.json`, 'utf8'));
rmSync(w, { recursive: true });
let fails = 0;
// want: [verdict, accepted, statement present]
const t = (name, [verdict, accepted, stmt], r) => {
  const ok = r.verdict === verdict && r.accepted === accepted && (r.statement != null) === stmt;
  if (!ok) fails++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict} accepted=${r.accepted} statement=${r.statement != null}${ok ? '' : ' ' + JSON.stringify(r.checks.filter((c) => !c.ok)).slice(0, 200)}`);
  return r;
};
const FAIL = ['fail', false, false], DEMO = ['demo', false, false], SHOWN = ['demo', false, true];
t('core 0..200 alone (DEMO: below minimum work): statement withheld', DEMO, v.verifyDeposit({ bundle: core, ask: { txid: T, vout: 0, amount: 1000000000, expect: '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320' } }));
const r = t('core 0..200 alone, allow_demo: the statement, still not accepted', SHOWN, v.verifyDeposit({ bundle: core, ask: { txid: T, vout: 0, amount: 1000000000, min_depth: 31, allow_demo: true } }));
if (r.statement?.confirmations !== 31 || r.statement?.validated_through !== 200 || r.statement?.output?.amount !== 1000000000) { fails++; console.log('FAIL statement', r.statement); }
const s = t('bound to the header chain 0..4,031, tip expected, allow_demo', SHOWN, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, min_depth: 6, expect: TIP, allow_demo: true } })).statement;
if (s?.confirmations !== 3862 || s?.validated_confirmations !== 31 || !s?.block_fully_validated || s?.tip_height !== 4031) { fails++; console.log('FAIL statement', s); }
if (!v.verifyDeposit({ bundle: tip, ask: { txid: T, expect: TIP } }).identity_checked) { fails++; console.log('FAIL identity_checked'); }
t('the same without expect: not accepted, statement withheld', DEMO, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, min_depth: 6 } }));
t('the same, require_identity: false', DEMO, v.verifyDeposit({ bundle: tip, ask: { txid: T, require_identity: false } }));
t('require_identity not a boolean', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, require_identity: 'no' } }));
t('depth 3,862, the tip expected', DEMO, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, amount: 1000000000, min_depth: 3862, expect: TIP } }));
t('depth 3,863', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_depth: 3863, expect: TIP } }));
t('validated depth 31', DEMO, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_validated_depth: 31, expect: TIP } }));
t('validated depth 32', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_validated_depth: 32 } }));
t('another txid asked', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: 'b1fea52486ce0c62bb442b530a3f0132b826c74e473d1f2c220bfa78111c5082' } }));
t('wrong amount', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, amount: 1 } }));
t('the header runs without their key', FAIL, v0.verifyDeposit({ ...KEYS, headersVk: undefined, headersPin: undefined, bundle: tip, ask: { txid: T } }));
t('another core pin', FAIL, v.verifyDeposit({ corePin: KEYS.headersPin, bundle: core, ask: { txid: T } }));
t('no core image (no engine image is published yet)', FAIL, v0.verifyDeposit({ bundle: core, ask: { txid: T } }));
const bad = structuredClone(tip); bad.core_tip_mmr_path[0] = '00'.repeat(32);
t('binding path tampered', FAIL, v.verifyDeposit({ bundle: bad, ask: { txid: T } }));
const bb = structuredClone(tip); bb.block.branch[0] = '11'.repeat(32);
t('merkle branch tampered', FAIL, v.verifyDeposit({ bundle: bb, ask: { txid: T } }));
t('no txid asked', FAIL, v.verifyDeposit({ bundle: tip, ask: {} }));
t('not a bundle', FAIL, v.verifyDeposit({ bundle: '{"kind":1}', ask: { txid: T } }));
t('misspelt ask key (ammount)', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, ammount: 1, expect: TIP } }));
t('misspelt ask key (min_dept)', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_dept: 3863, expect: TIP } }));
for (const [k, x] of [['amount', parseInt('ten btc')], ['min_depth', parseInt('')], ['max_tip_age', Infinity], ['script', null], ['op_return', null]]) {
  const q = t(`ask ${k}: ${x} (null in JSON)`, FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, [k]: x, expect: TIP } }));
  if (!q.checks.some(c => !c.ok && c.what.startsWith(`ask: ${k} is null`))) { fails++; console.log(`FAIL no check names ${k}`, q.checks); }
}
const xf = structuredClone(tip); xf.block.extra = 1;
t('bundle with an unknown field', FAIL, v.verifyDeposit({ bundle: xf, ask: { txid: T, expect: TIP } }));
const xm = structuredClone(tip); xm.block.mmr_path = [];
t('bundle with an mmr_path on a fully validated block', FAIL, v.verifyDeposit({ bundle: xm, ask: { txid: T, expect: TIP } }));
console.log(fails ? `test_deposit_wasm: ${fails} FAILED` : 'test_deposit_wasm: all as expected');
process.exit(fails ? 1 : 0);
