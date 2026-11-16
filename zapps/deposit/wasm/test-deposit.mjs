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
// want: [verdict, accepted, statement present]
const t = (name, [verdict, accepted, stmt], r) => {
  const ok = r.verdict === verdict && r.accepted === accepted && (r.statement != null) === stmt;
  if (!ok) fails++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict} accepted=${r.accepted} statement=${r.statement != null}`);
  return r;
};
const FAIL = ['fail', false, false];
// review (11-15): a DEMO statement is withheld unless asked for, and never accepted
t('core 0..200 alone (DEMO: below minimum work): statement withheld', ['demo', false, false], v.verifyDeposit({ bundle: core, ask: { txid: T, vout: 0, amount: 1000000000, expect: `200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320` } }));
const r = t('core 0..200 alone, allow_demo: the statement, still not accepted', ['demo', false, true], v.verifyDeposit({ bundle: core, ask: { txid: T, vout: 0, amount: 1000000000, min_depth: 31, allow_demo: true } }));
if (r.statement?.confirmations !== 31 || r.statement?.validated_through !== 200 || r.statement?.output?.amount !== 1000000000) { fails++; console.log('FAIL statement', r.statement); }
const s = t('bound to the header chain 0..975,278, tip expected: accepted', ['ok', true, true], v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, min_depth: 6, expect: TIP } })).statement;
if (!v.verifyDeposit({ bundle: tip, ask: { txid: T, expect: TIP } }).identity_checked) { fails++; console.log('FAIL identity_checked'); }
// review (11-15): without a tip source the verdict is at best DEMO (require_identity, on by default)
t('the same without expect: not accepted, statement withheld', ['demo', false, false], v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, min_depth: 6 } }));
t('the same, require_identity: false (the caller says so)', ['ok', true, true], v.verifyDeposit({ bundle: tip, ask: { txid: T, require_identity: false } }));
t('require_identity not a boolean', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, require_identity: 'no' } }));
if (s?.confirmations !== 975109 || s?.validated_confirmations !== 31 || !s?.block_fully_validated || s?.tip_height !== 975278) { fails++; console.log('FAIL statement', s); }
t('another txid asked', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: 'b1fea52486ce0c62bb442b530a3f0132b826c74e473d1f2c220bfa78111c5082' } }));
t('wrong amount', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, vout: 0, amount: 1 } }));
t('validated depth 0, explicit (the block is fully validated)', ['ok', true, true], v.verifyDeposit({ bundle: tip, ask: { txid: T, min_validated_depth: 0, expect: TIP } }));
t('validated depth 32', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_validated_depth: 32 } }));
t('depth 975,110', FAIL, v.verifyDeposit({ bundle: tip, ask: { txid: T, min_depth: 975110 } }));
const bad = structuredClone(tip); bad.core_tip_mmr_path[0] = '00'.repeat(32);
t('binding path tampered', FAIL, v.verifyDeposit({ bundle: bad, ask: { txid: T } }));
const bb = structuredClone(tip); bb.block.branch[0] = '11'.repeat(32);
t('merkle branch tampered', FAIL, v.verifyDeposit({ bundle: bb, ask: { txid: T } }));
t('no txid asked', FAIL, v.verifyDeposit({ bundle: tip, ask: {} }));
t('not a bundle', FAIL, v.verifyDeposit({ bundle: '{"kind":1}', ask: { txid: T } }));
console.log(fails ? `test_deposit_wasm: ${fails} FAILED` : 'test_deposit_wasm: all as expected');
process.exit(fails ? 1 : 0);
