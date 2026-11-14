// verifyHeaders (the header-chain zyncAPP) in the WebAssembly verifier, on the same real chained receipts as
// test_headers_verify.sh, and its MMR inclusion cases. Usage: node test_headers_wasm.mjs <swiftsync-verify.js> <swiftsync_verify_wasm.wasm>
// HEADERS_IMAGE=<hex>: verify against this image id instead of the built-in one (receipts proved by a local build).
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const [js, wasm] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
const { loadVerifier } = await import(pathToFileURL(resolve(js)).href);
const v0 = await loadVerifier(readFileSync(wasm));
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
if (inc.included !== true || !/^[0-9a-f]{64}$/.test(inc.mmr_root || '')) { fails++; console.log('FAIL included / mmr_root', inc.included, inc.mmr_root); }
t('block 37 at height 38', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 38, hash: H37, path: P37 } }));
t("block 38's hash on 37's path", 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H38, path: P37 } }));
t('a truncated path', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: P37.slice(0, -1) } }));
t('an extended path', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: [...P37, P37[0]] } }));
t('a path that is not hex', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 37, hash: H37, path: ['zz'] } }));
t('tx in block 170', 'demo', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: TX } }));
t('  at the wrong position', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: { ...TX, pos: 0 } } }));
t("  under block 171's header", 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: { ...TX, header: hdr(171) } } }));
t('  a branch deeper than the coinbase\'s', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: { ...TX, branch: [...TX.branch, ...TX.branch] } } }));
t('  another block\'s coinbase', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: { ...TX, coinbase: OTHER.coinbase, coinbase_branch: OTHER.coinbase_branch } } }));
// the 64-byte forgery: block 170's two txids concatenated hash to its merkle root
const rev = (h) => h.match(/../g).reverse().join('');
const inner = rev(TX.branch[0]) + rev(TX.txid);
const root = TX.header.slice(72, 136);
t('  a 64-byte inner node as a transaction', 'fail', v.verifyHeaders({ receipts: [a, b], include: { height: 170, hash: H170, path: P170, tx: { ...TX, raw: inner, txid: rev(root), pos: 0, branch: [] } } }));
const none = v.verifyHeaders({ receipts: [a, b] });
if (none.included !== null) { fails++; console.log('FAIL included without include', none.included); }
console.log(fails ? `WASM_HEADERS: ${fails} FAILED` : 'WASM_HEADERS_OK');
process.exit(fails ? 1 : 0);
