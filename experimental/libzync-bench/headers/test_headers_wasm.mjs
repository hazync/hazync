// verifyHeaders (the header-chain zyncAPP) in the WebAssembly verifier, on the same real chained receipts as
// test_headers_verify.sh. Usage: node test_headers_wasm.mjs <swiftsync-verify.js> <swiftsync_verify_wasm.wasm>
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const [js, wasm] = process.argv.slice(2);
const here = dirname(fileURLToPath(import.meta.url));
const { loadVerifier } = await import(pathToFileURL(resolve(js)).href);
const v = await loadVerifier(readFileSync(wasm));
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
console.log(fails ? `WASM_HEADERS: ${fails} FAILED` : 'WASM_HEADERS_OK');
process.exit(fails ? 1 : 0);
