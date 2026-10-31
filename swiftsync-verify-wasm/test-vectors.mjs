// node test-vectors.mjs: run ../swiftsync-verify/vectors/manifest.json through the WASM build and compare
// each verdict with the case's expected exit code (0 ok, 2 demo, 1 fail). Exit 1 on any difference.
import fs from 'node:fs';
import path from 'node:path';
import { loadVerifier } from './swiftsync-verify.js';
const dir = new URL('../swiftsync-verify/vectors/', import.meta.url).pathname;
const v = await loadVerifier(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm', import.meta.url)));
const m = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json')));
const code = { ok: 0, demo: 2, fail: 1 };
let bad = 0;
for (const c of m.cases) {
  const a = { network: 'mainnet', files: [] };
  for (let i = 0; i < c.args.length; i++) {
    const x = c.args[i];
    if (x === '--network') a.network = c.args[++i];
    else if (x === '--image') a.imageHex = c.args[++i];
    else if (x === '--anchor') a.anchorJson = fs.readFileSync(path.join(dir, c.args[++i]), 'utf8');
    else a.files.push(x);
  }
  const r = v.verify({ network: a.network, imageHex: a.imageHex, anchorJson: a.anchorJson,
    receipt: fs.readFileSync(path.join(dir, a.files[0])), hints: fs.readFileSync(path.join(dir, a.files[1])) });
  const ok = code[r.verdict] === c.expect_exit;
  bad += !ok;
  console.log(`${ok ? 'PASS' : 'WRONG'}  ${c.name.padEnd(20)} ${r.verdict} (want exit ${c.expect_exit})`);
}
console.log(`${m.cases.length - bad}/${m.cases.length} cases as expected (wasm)`);
process.exit(bad ? 1 : 0);
