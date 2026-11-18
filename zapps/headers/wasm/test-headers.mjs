// verifyHeaders in the header-chain zAPP's own WASM module (zapp-headers.js; until 15.11.2026 in the core verifier's),
// on the same real chained receipts as ../test_headers_verify.sh, and its MMR inclusion cases. Usage: node
// test-headers.mjs [<zapp-headers.js> <zapp_headers_wasm.wasm>] (default: this directory's glue and its build).
// The receipts and paths are the experiment's (experimental/libzync-bench/headers).
// HEADERS_IMAGE=<hex>: verify against this image id instead of the built-in one (receipts proved by a local build).
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const own = dirname(fileURLToPath(import.meta.url));
const [js = `${own}/zapp-headers.js`, wasm = `${own}/target/wasm32-unknown-unknown/release/zapp_headers_wasm.wasm`] = process.argv.slice(2);
const here = resolve(own, '../../../experimental/libzync-bench/headers');
const { loadHeaders } = await import(pathToFileURL(resolve(js)).href);
const v0 = await loadHeaders(readFileSync(wasm));
const imageHex = process.env.HEADERS_IMAGE || '';
const v = { verifyHeaders: (args) => v0.verifyHeaders({ imageHex, ...args }) };
const a = new Uint8Array(readFileSync(`${here}/hdr_0_100.rcpt`)), b = new Uint8Array(readFileSync(`${here}/hdr_101_200.rcpt`));
const H = '000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
let fails = 0;
const t = (name, want, r) => { const ok = r.verdict === want; if (!ok) fails++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict}`); };
const r = v.verifyHeaders({ receipts: [a, b], expect: `200:${H}` });
t('genesis..100 + 101..200, tip expected (DEMO: below minimum work)', 'demo', r);
if (r.height !== 200 || r.hash !== H || !r.identity_checked) { fails++; console.log('FAIL height / hash / identity_checked', r.height, r.hash, r.identity_checked); }
t('wrong order', 'fail', v.verifyHeaders({ receipts: [b, a] }));
t('a run that does not start from genesis', 'fail', v.verifyHeaders({ receipts: [b] }));
const bad = b.slice(); bad[bad.length >> 1] ^= 1;
t('one bit flipped', 'fail', v.verifyHeaders({ receipts: [a, bad] }));
t('wrong expected tip', 'fail', v.verifyHeaders({ receipts: [a, b], expect: `200:${H.slice(0, -1)}1` }));
// the whole chain 0..975,278 (Core's minimum work): 'ok' only with its tip expected; without, 'demo' (review 11-16)
{
  const tipR = new Uint8Array(readFileSync(`${here}/headers_0_tip.rcpt`));
  const TIP = '975278:00000000000000000001d951592f7a06f0d24697e4f9c362ae173fced55583e3';
  const w = v.verifyHeaders({ receipts: [tipR], expect: TIP });
  t('the whole chain, tip expected', 'ok', w);
  if (!w.identity_checked) { fails++; console.log('FAIL identity_checked on the whole chain'); }
  const n = v.verifyHeaders({ receipts: [tipR] });
  t('the whole chain, no expect: not shown to be the network\'s', 'demo', n);
  if (n.identity_checked) { fails++; console.log('FAIL identity_checked without expect'); }
}
// MMR inclusion: block 37 by its path to the tip 200; a transaction of block 170 by its header and merkle branch
const lines = (f) => readFileSync(`${here}/${f}`, 'utf8').split('\n').filter((l) => l.trim());
const P37 = lines('mmr_path_37_tip200.txt'), P170 = lines('mmr_path_170_tip200.txt');
const H37 = '00000000ddd96d128e122d1179034ff66c27dc583eb9e8996a0b1779c60c6f86';
const H38 = '000000007c19ee8e924d3024d58efff50f872aadf256140bb1e3d62fea4fd6dd';
const H170 = '00000000d1145790a8694403d4063f323d499e655c83426834d4ce2f8dd4a2ee';
const hdr = (h) => lines('h0_200.txt').find((l) => l.startsWith(`${h} `)).split(' ')[1];
const TX = JSON.parse(readFileSync(`${here}/tx/tx_170_1.json`, 'utf8'));
const OTHER = JSON.parse(readFileSync(`${here}/tx/tx_100003_0.json`, 'utf8'));
const inc = v.verifyHeaders({ receipts: [a, b], expect: `200:${H}`, include: { height: 37, hash: H37, path: P37 } });
t('block 37 is on the chain (MMR path)', 'demo', inc);
// a DEMO chain (below the minimum work: anyone can mine a difficulty-1 fork) never says included: true
const canon = (x) => JSON.stringify(x, (k, v) => (v && typeof v === 'object' && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort()) : v));
const same = (x, y) => canon(x) === canon(y);
const want = (name, got, exp) => { const ok = same(got, exp); if (!ok) fails++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${JSON.stringify(got)}`); };
if (!/^[0-9a-f]{64}$/.test(inc.mmr_root || '')) { fails++; console.log('FAIL mmr_root', inc.mmr_root); }
want('  on a DEMO chain: included false, included_demo true, inclusion names the block',
     [inc.included, inc.included_demo, inc.inclusion], [false, true, { height: 37, hash: H37, txid: null }]);
t('block 37 at height 38', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 38, hash: H37, path: P37 } }));
t("block 38's hash on 37's path", 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H38, path: P37 } }));
t('a truncated path', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: P37.slice(0, -1) } }));
t('an extended path', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: [...P37, P37[0]] } }));
t('a path that is not hex', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: ['zz'] } }));
const T170 = 'f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16';   // the transaction ASKED about
const tx170 = (tx, txid = T170) => v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid, tx } });
const ok170 = tx170(TX);
t('tx in block 170', 'demo', ok170);
want('  included false (DEMO), included_demo true, inclusion names the block and the ASKED txid',
     [ok170.included, ok170.included_demo, ok170.inclusion], [false, true, { height: 170, hash: H170, txid: T170 }]);
t("  the answer's own txid field is not read (wrong, the right raw: still demo)", 'demo', tx170({ ...TX, txid: '00'.repeat(32) }));
// a lying service: asked about T170, it answers with a VALID proof of ANOTHER transaction (block 170's coinbase)
const CB170 = JSON.parse(readFileSync(`${here}/tx/tx_170_0.json`, 'utf8'));
const lie = tx170(CB170);
t('  a lying service: a valid proof of another transaction (the coinbase)', 'fail', lie);
want('    nothing included', [lie.included, lie.included_demo, lie.inclusion], [false, false, null]);
if (!lie.checks.some((c) => !c.ok && /not the transaction asked about/.test(c.what))) { fails++; console.log('FAIL the lie is not refused as another transaction'); }
t('  ...its txid field claiming the asked one', 'fail', tx170({ ...CB170, txid: T170 }));
t('  tx without the asked txid', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: TX } }));
t('  the asked txid without tx', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: T170 } }));
t('  an asked txid with a sign (+f...)', 'fail', tx170(TX, `+${T170.slice(1)}`));
t('  at the wrong position', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: T170, tx: { ...TX, pos: 0 } } }));
t("  under block 171's header", 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: T170, tx: { ...TX, header: hdr(171) } } }));
t('  a branch deeper than the coinbase\'s', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: T170, tx: { ...TX, branch: [...TX.branch, ...TX.branch] } } }));
t('  another block\'s coinbase', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: T170, tx: { ...TX, coinbase: OTHER.coinbase, coinbase_branch: OTHER.coinbase_branch } } }));
// the 64-byte forgery: block 170's two txids concatenated hash to its merkle root
const rev = (h) => h.match(/../g).reverse().join('');
const inner = rev(TX.branch[0]) + rev(TX.txid);
const root = TX.header.slice(72, 136);
t('  a 64-byte inner node as a transaction', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, txid: rev(root), tx: { ...TX, raw: inner, pos: 0, branch: [] } } }));
const none = v.verifyHeaders({ receipts: [a, b] });
want('nothing asked: included, included_demo and inclusion are null', [none.included, none.included_demo, none.inclusion], [null, null, null]);
// The frame itself, by hand (the raw exports, not the glue): the glue's bytes pass, a byte past the end and a frame
// cut short are refused before anything is verified.
const ex = (await WebAssembly.instantiate(readFileSync(wasm), {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_headers(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const frame = (include) => {
  const parts = [a, b, enc.encode(process.env.HEADERS_IMAGE || ''), enc.encode(`200:${H}`), enc.encode(include)];
  const fr = new Uint8Array(12 + parts.reduce((n, p) => n + 4 + p.length, 0));
  const dv = new DataView(fr.buffer);
  dv.setBigUint64(0, BigInt(Math.floor(Date.now() / 1000)), true);
  dv.setUint32(8, 2, true);
  let o = 12;
  for (const p of parts) { dv.setUint32(o, p.length, true); fr.set(p, o + 4); o += 4 + p.length; }
  return fr;
};
const fr = frame('');
const trailing = new Uint8Array(fr.length + 1); trailing.set(fr);
for (const [name, bytes, want, why] of [['frame: the glue\'s bytes', fr, 'demo', ''],
                                        ['frame: a byte past the end', trailing, 'fail', 'trailing'],
                                        ['frame: cut short', fr.slice(0, fr.length - 1), 'fail', 'truncated']]) {
  const r = raw(bytes);
  const ok = r.verdict === want && (!why || r.checks.some((c) => !c.ok && c.what.includes(why)));
  if (!ok) fails++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict}`);
}
// Review 11-18: the include JSON takes only height, hash, path, txid, tx (and tx only the /tx answer's documented keys),
// each once. Before, every other key was ignored: txid and tx both spelt wrong verified as a block-only inclusion
// (included_demo true), and a key given twice kept its last value.
{
  const inc170 = (rest) => `{"height": 170, "hash": "${H170}", "path": ${JSON.stringify(P170)}${rest}}`;
  const txj = JSON.stringify(TX);
  for (const [name, j, want, why] of [
    ['include: the documented keys', inc170(`, "txid": "${T170}", "tx": ${txj}`), 'demo', ''],
    ['include: txid and tx both misspelt', inc170(`, "tixd": "${T170}", "txx": ${txj}`), 'fail', 'unknown field `tixd`'],
    ['include: an extra key', inc170(`, "txid": "${T170}", "tx": ${txj}, "extra": 1`), 'fail', 'unknown field `extra`'],
    ['include: a key twice', inc170(`, "height": 170`), 'fail', 'duplicate field `height`'],
    ['include: tx with an unknown key', inc170(`, "txid": "${T170}", "tx": ${JSON.stringify({ ...TX, posn: 1 })}`), 'fail', 'unknown field `posn`'],
    ['include: tx with a key twice', inc170(`, "txid": "${T170}", "tx": ${txj.replace('{', '{"pos": 0, ')}`), 'fail', 'duplicate field `pos`'],
    ['include: an array', `[170, "${H170}", ${JSON.stringify(P170)}]`, 'fail', 'include is not a JSON object'],
  ]) {
    const r = raw(frame(j));
    const ok = r.verdict === want && (!why || r.checks.some((c) => !c.ok && c.what.includes(why)));
    if (!ok) fails++;
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}: ${r.verdict}${ok ? '' : ' ' + JSON.stringify(r.checks).slice(0, 160)}`);
  }
}
console.log(fails ? `WASM_HEADERS: ${fails} FAILED` : 'WASM_HEADERS_OK');
process.exit(fails ? 1 : 0);
