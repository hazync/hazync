// verifyDeposit in the deposit-proof zAPP's own WASM module (zapp-deposit.js; until 15.11.2026 in the core verifier's), on
// bundles `zapp-deposit build` makes from the real receipts (as ../test_deposit.sh).
// Usage: node test-deposit.mjs <zapp-deposit> [<zapp-deposit.js> <zapp_deposit_wasm.wasm>] (default: this directory's).
import { readFileSync, mkdtempSync, rmSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const own = dirname(fileURLToPath(import.meta.url));
const [zd, js = `${own}/zapp-deposit.js`, wasm = `${own}/target/wasm32-unknown-unknown/release/zapp_deposit_wasm.wasm`] = process.argv.slice(2);
const root = resolve(own, '../../..');
const here = `${root}/experimental/libzync-bench/deposit`;
const { loadDeposit } = await import(pathToFileURL(resolve(js)).href);
const v = await loadDeposit(readFileSync(wasm));
const T = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';
const TIP = '975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3';
const w = mkdtempSync(join(tmpdir(), 'deposit-wasm-'));
const make = (out, extra) => execFileSync(resolve(zd), ['build', '--core', `${root}/swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt`,
  '--history', `${here}/history_0_200.bin`, '--height', '170', '--block', `${here}/block_170_raw.txt`, '--txid', T, ...extra, '-o', out],
  { stdio: ['ignore', 'ignore', 'ignore'] });
make(`${w}/core.json`, []);
make(`${w}/tip.json`, ['--headers', `${root}/experimental/libzync-bench/headers/headers_0_tip.rcpt`, '--tip-path', `${here}/mmr_path_200_tip975278.txt`]);
const core = readFileSync(`${w}/core.json`, 'utf8'), tip = JSON.parse(readFileSync(`${w}/tip.json`, 'utf8'));
rmSync(w, { recursive: true });
let fails = 0;
const t = (name, want, r) => { const ok = r.verdict === want && (want === 'fail') === (r.statement == null); if (!ok) fails++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict}`); return r; };
const r = t('core 0..200 alone, output 0 = 10 BTC (DEMO: below minimum work)', 'demo', v.verifyDeposit({ bundle: core, ask: { txid: T, vout: 0, amount: 1000000000, min_depth: 31 } }));
if (r.statement?.confirmations !== 31 || r.statement?.validated_through !== 200 || r.statement?.output?.amount !== 1000000000) { fails++; console.log('FAIL statement', r.statement); }
const s = t('bound to the header chain 0..975,278, tip expected', 'ok', v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, min_depth: 6, expect: TIP } })).statement;
if (s?.confirmations !== 975109 || s?.validated_confirmations !== 31 || !s?.block_fully_validated || s?.tip_height !== 975278) { fails++; console.log('FAIL statement', s); }
t('another txid asked', 'fail', v.verifyDeposit({ bundle: tip, ask: { txid: 'b1fea52486ce0c62bb442b530a3f0132b826c74e473d1f2c220bfa78111c5082' } }));
t('wrong amount', 'fail', v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, amount: 1 } }));
t('validated depth 32', 'fail', v.verifyDeposit({ bundle: tip, ask: { txid: T, min_validated_depth: 32 } }));
t('depth 975,110', 'fail', v.verifyDeposit({ bundle: tip, ask: { txid: T, min_depth: 975110 } }));
const bad = structuredClone(tip); bad.core_tip_mmr_path[0] = '00'.repeat(32);
t('binding path tampered', 'fail', v.verifyDeposit({ bundle: bad, ask: { txid: T } }));
const bb = structuredClone(tip); bb.block.branch[0] = '11'.repeat(32);
t('merkle branch tampered', 'fail', v.verifyDeposit({ bundle: bb, ask: { txid: T } }));
t('no txid asked', 'fail', v.verifyDeposit({ bundle: tip, ask: {} }));
t('not a bundle', 'fail', v.verifyDeposit({ bundle: '{"kind":1}', ask: { txid: T } }));
console.log(fails ? `test_deposit_wasm: ${fails} FAILED` : 'test_deposit_wasm: all as expected');
process.exit(fails ? 1 : 0);
