// node test-zeroconf.mjs <tests/data/engine>: the WASM zeroconf verifier on the zAPP's real engine proof (regtest tip
// 110, real children: ../tests/data/engine/README.md). The fixture is a REGTEST proof and a release module has no regtest
// image (the tests-only regtest-test feature is refused in any WASM graph: scripts/wasm-build.sh), so it is asked as
// mainnet: the real STARK, the journal, the statement and every policy check run, and the ONLY check that fails is the
// network (the core profile's regtest bit). Each refusal case must add its own failing check on top of that one, and
// keep the proof itself verified where the case does not touch it. The OK / DEMO verdicts on this proof are covered
// natively (../test.sh: the library and the CLI with regtest-test); here, every check the page relies on.
import fs from 'node:fs';
import { loadZeroconf } from './zapp-zeroconf.js';
const [d] = process.argv.slice(2);
const wasm = fs.readFileSync(new URL('./target/wasm32-unknown-unknown/release/zapp_zeroconf_wasm.wasm', import.meta.url));
const v = await loadZeroconf(wasm);
const rd = (f) => new Uint8Array(fs.readFileSync(`${d}/${f}`));
const T = JSON.parse(fs.readFileSync(`${d}/T.json`, 'utf8'));
const proof = rd('proof.hze'), vk = rd('vk.bin');
const pin = new Uint8Array(Buffer.from(fs.readFileSync(`${d}/vk.sha256`, 'utf8').trim(), 'hex'));
const imageHex = fs.readFileSync(`${d}/image_id`, 'utf8').trim(), coreImageHex = fs.readFileSync(`${d}/core_image_id`, 'utf8').trim();
const anchor = fs.readFileSync(`${d}/anchor_state_109.json`, 'utf8');
const TIP = '110:2d5cc9c833f072d2744d26a9354e94111e77d05669b5004f7d68641c42ae9d53';
const base = { proof, txid: T.txid, wtxid: T.wtxid, expectTip: TIP, anchor, imageHex, coreImageHex, vk, pin };
const NET = 'core profile 0xf9 == the image';          // the one expected failure: a regtest proof asked as mainnet
const PROOF_OK = 'the engine proof verifies against the zeroconf image';
const flipped = proof.slice(); flipped[proof.length >> 1] ^= 1;
const otherKey = vk.slice(); otherKey[100] ^= 1;
const swap = (h) => h.replace(/[0-9a-f]/, (c) => (c === '0' ? '1' : '0'));
// [name, args, the failing checks expected (substrings; NET counts when the statement got that far), proof verified?]
const cases = [
  ['the real proof, its txid, wtxid and tip', base, [NET], true],
  ['no wtxid, no tip', { ...base, wtxid: '', expectTip: '' }, [NET], true],
  ['another txid', { ...base, txid: swap(T.txid) }, ['the proven transaction is', NET], true],
  ['another wtxid (review R2-6 #3)', { ...base, wtxid: swap(T.wtxid) }, ["the proven transaction's wtxid", NET], true],
  ['another tip', { ...base, expectTip: TIP.replace(/d53$/, 'd54') }, ['the tip is block', NET], true],
  ['another tip height', { ...base, expectTip: TIP.replace(/^110/, '111') }, ['the tip is block', NET], true],
  ['genesis claimed (no anchor)', { ...base, anchor: undefined }, ['anchor', NET], true],
  ['another core image', { ...base, coreImageHex: imageHex }, ['rests on the trusted', NET], true],
  ['a flipped proof byte', { ...base, proof: flipped }, [PROOF_OK], false],
  ['another image id', { ...base, imageHex: coreImageHex }, [PROOF_OK], false],
  ['a key that is not the pinned one', { ...base, vk: otherKey }, [PROOF_OK], false],
  ['the key with another pin', { ...base, pin: new Uint8Array(32) }, [PROOF_OK], false],
  ['a 31-byte pin', { ...base, pin: pin.slice(1) }, ["the key's pin is 32 bytes"], false],
  ['no key (no engine image is published yet)', { ...base, vk: undefined }, ['no engine zeroconf image is published yet'], false],
  ['no txid', { ...base, txid: '' }, ['give the txid'], false],
  ['a malformed tip', { ...base, expectTip: '110:xyz' }, ['expect: want'], false],
  ['a malformed anchor', { ...base, anchor: '{"not": "a state"}' }, ['anchor is not a ChainState'], false],
  ['signet asked of a regtest proof', { ...base, network: 'signet' }, ['core profile 0xf9 =='], true],
];
let bad = 0;
for (const [n, a, wantFail, proofOk] of cases) {
  const r = v.verifyZeroconf(a);
  const failed = r.checks.filter((c) => !c.ok).map((c) => c.what);
  const matched = wantFail.every((w) => failed.some((f) => f.includes(w))) && failed.every((f) => wantFail.some((w) => f.includes(w)));
  const pv = r.checks.some((c) => c.ok && c.what.includes(PROOF_OK));
  const ok = r.verdict === 'fail' && matched && pv === proofOk;
  bad += !ok;
  console.log(`${ok ? 'PASS' : 'WRONG'} ${n}: ${r.verdict}, proof ${pv ? 'verified' : 'not verified'}, failing: ${failed.map((f) => f.slice(0, 60)).join(' | ')}`);
}
// The frame itself, by hand (the raw exports, not the glue): the glue's bytes reach the checks, a byte past the end, a
// frame cut short and an unknown network code are refused before anything is verified.
const ex = (await WebAssembly.instantiate(wasm, {})).instance.exports;
const raw = (bytes) => {
  const p = ex.alloc(bytes.length);
  new Uint8Array(ex.memory.buffer, p, bytes.length).set(bytes);
  const r = ex.verify_zeroconf(p, bytes.length);
  const n = new DataView(ex.memory.buffer).getUint32(r, true);
  return JSON.parse(new TextDecoder().decode(new Uint8Array(ex.memory.buffer, r + 4, n)));
};
const enc = new TextEncoder();
const parts = [proof, enc.encode(anchor), enc.encode(imageHex), enc.encode(coreImageHex), enc.encode(T.txid), enc.encode(T.wtxid), enc.encode(TIP), vk, pin];
const frame = new Uint8Array(9 + parts.reduce((n, p) => n + 4 + p.length, 0));
new DataView(frame.buffer).setBigUint64(1, BigInt(Math.floor(Date.now() / 1000)), true);
let o = 9;
for (const p of parts) { new DataView(frame.buffer).setUint32(o, p.length, true); frame.set(p, o + 4); o += 4 + p.length; }
const trailing = new Uint8Array(frame.length + 1); trailing.set(frame);
const net7 = frame.slice(); net7[0] = 7;
for (const [n, bytes, why] of [['the frame as the glue writes it', frame, NET],
                               ['a byte past the end of the frame', trailing, 'trailing'],
                               ['a frame cut short', frame.slice(0, frame.length - 1), 'truncated'],
                               ['an unknown network code', net7, 'unknown network code 7']]) {
  const r = raw(bytes);
  const failed = r.checks.filter((c) => !c.ok);
  const ok = r.verdict === 'fail' && failed.length === 1 && failed[0].what.includes(why);
  bad += !ok;
  cases.push([n]);
  console.log(`${ok ? 'PASS' : 'WRONG'} ${n}: ${r.verdict} (${(failed[0] || {}).what})`);
}
const id = v.zeroconfImageId();
bad += id !== '';
cases.push(['no built-in image id yet']);
console.log(`${id === '' ? 'PASS' : 'WRONG'} no built-in image id yet: ${JSON.stringify(id)}`);
console.log(`ZEROCONF_WASM ${cases.length - bad}/${cases.length}`);
process.exit(bad ? 1 : 0);
