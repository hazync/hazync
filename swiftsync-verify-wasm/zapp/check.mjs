// The coin-check page's logic, run under Node against a live server (zapp/serve.py): verify the proof, ask for a
// script's balance and history, check each against the proof; then TAMPER with an answer and require the check to
// refuse it (the server is untrusted: a lie must not pass).
//   node zapp/check.mjs http://127.0.0.1:8600 <scriptPubKey hex> [<scriptPubKey hex> ...]
import { readFileSync } from 'node:fs';
import { loadVerifier } from '../swiftsync-verify.js';

const [base, ...spks] = process.argv.slice(2);
const v = await loadVerifier(readFileSync(new URL('../swiftsync_verify_wasm.wasm', import.meta.url)));
const receipt = new Uint8Array(await (await fetch(`${base}/zapp/proof.rcpt`)).arrayBuffer());
const args = { network: 'mainnet', image: 'product', receipt };
const r = v.verify(args);
console.log(`proof: ${r.verdict.toUpperCase()}, genesis to ${r.height}, tip ${r.tip}, ${r.utxo_leaves} coins ever`);
let bad = r.verdict === 'fail' ? 1 : 0;
for (const spk of spks) {
  for (const kind of ['balance', 'history']) {
    const fact = await (await fetch(`${base}/zapp/api/trie/${kind}/${spk}`)).json();
    const c = v.verifyFacts({ ...args, facts: { tries: [fact] } });
    console.log(`  ${kind} ${spk.slice(0, 16)}...: ${c.verdict.toUpperCase()} - ${c.facts?.[0]?.what}`);
    if (c.verdict === 'fail') bad++;
    // a lying server: flip one word of the trie it sent
    const w = fact.proof;
    const i = 8 * Math.floor(w.length / 16);
    const lie = { ...fact, proof: w.slice(0, i) + (w[i] === '0' ? '1' : '0') + w.slice(i + 1) };
    const t = v.verifyFacts({ ...args, facts: { tries: [lie] } });
    console.log(`    tampered ${kind}: ${t.verdict.toUpperCase()} (want FAIL)`);
    if (t.verdict !== 'fail') bad++;
  }
}
console.log(bad ? `CHECK_FAILED (${bad})` : 'CHECK_OK');
process.exit(bad ? 1 : 0);
