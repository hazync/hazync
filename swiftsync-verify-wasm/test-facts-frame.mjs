// node test-facts-frame.mjs: verify_facts' frame (regression, 15.11.2026). From f713e3a6 the optional lineage chunk was
// read straight after the image id in verify_facts too, so the facts JSON was taken for a lineage: every verifyFacts
// call with facts failed, and so did verify with an expected tip (it goes through verify_facts). Real receipt
// mainnet 1..200 (image 8888d59c) and the real block-9 balance trie fact. Exit 1 on any wrong answer.
import fs from 'node:fs';
import { loadVerifier } from './swiftsync-verify.js';
const v = await loadVerifier(fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/swiftsync_verify_wasm.wasm', import.meta.url)));
const rcpt = fs.readFileSync(new URL('../swiftsync-verify/vectors/mainnet_1_200_8888d59c.rcpt', import.meta.url));
const fact = JSON.parse(fs.readFileSync(new URL('../experimental/libzync-bench/zapp/fact_balance_block9_200.json', import.meta.url), 'utf8'));
const TIP = '200:000000008f1a7008320c16b8402b7f11e82951f44ca2663caf6860ab2eeef320';
const base = { network: 'mainnet', image: 'product', receipt: rcpt };
let bad = 0;
const check = (name, ok, got) => { bad += !ok; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}  ${got}`); };

let r = v.verifyFacts({ ...base, facts: { tries: [fact] } });
// A 200-block chain is below Core's minimum chainwork, so the verdict is DEMO and a fact that holds reports ok:false with
// "holds on this chain, but the chain is not shown to be the network's" (ZYNCAPPS.md §8) -- an answer, not a frame error.
const holds = (r) => r.facts?.length === 1 && /^holds on this chain/.test(r.facts[0].what);
check('verifyFacts with a real balance fact: demo, the fact holds on this chain', r.verdict === 'demo' && holds(r), `${r.verdict} ${JSON.stringify(r.facts ?? r.error)}`);
r = v.verifyFacts({ ...base, facts: { tries: [fact] }, expect: TIP });
check('verifyFacts with the fact and the expected tip', r.verdict !== 'fail' && holds(r), `${r.verdict} ${JSON.stringify(r.facts ?? r.error)}`);
r = v.verify({ ...base, expect: TIP });
check('verify with an expected tip (the facts path, no facts)', r.verdict !== 'fail', `${r.verdict} ${r.error ?? ''}`);
r = v.verify({ ...base, expect: '200:' + '00'.repeat(32) });
// the refusal must be the tip comparison itself, not a frame error (the old wasm also gave 'fail', for the wrong reason)
check('a wrong expected tip is refused, by the tip check', r.verdict === 'fail' && r.facts?.some((f) => !f.ok && /not the proof's tip/.test(f.what)),
  `${r.verdict} ${JSON.stringify(r.facts ?? r.error)}`);
const bent = { ...fact, script_pubkey: fact.script_pubkey.replace(/.$/, (c) => (c === '0' ? '1' : '0')) };
r = v.verifyFacts({ ...base, facts: { tries: [bent] } });
// on a DEMO chain a fact that HOLDS also reports ok:false, so assert it does not hold
check('a fact for another script is refused', r.facts?.length === 1 && r.facts[0].ok === false && !holds(r), `${JSON.stringify(r.facts ?? r.error)}`);
console.log(bad ? `${bad} FAILED` : 'FACTS_FRAME_OK');
process.exit(bad ? 1 : 0);
