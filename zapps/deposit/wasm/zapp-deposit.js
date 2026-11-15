// Glue for the deposit-proof zyncAPP's WASM module (docs/libzync/DEPOSIT_ZAPP.md), a zAPP module of its own
// (docs/libzync/ZYNCAPPS.md §9), not the core verifier's hazync-verify.js (until 15.11.2026 verifyDeposit was there).
// Hand-written and short on purpose, as the core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadDeposit } from './zapp-deposit.js';
//   const z = await loadDeposit(fetch('./zapp-deposit.wasm'));
//   const r = z.verifyDeposit({ bundle, ask: { txid, vout, amount, min_depth, expect } });
//   r.accepted // THE field to gate on: verdict 'ok' and the tip compared with expect (require_identity, on by default)
//   r.verdict  // 'ok' | 'demo' | 'fail'; r.statement (null unless 'ok', or 'demo' with ask.allow_demo)

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadDeposit(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_deposit, memory } = instance.exports;
  return {
    /** The deposit-proof zyncAPP (docs/libzync/DEPOSIT_ZAPP.md): bundle = the JSON zapp-deposit build writes (string or
     *  object); ask = { txid (the one YOU ask about), vout?, amount?, script?, op_return?, min_depth?,
     *  min_validated_depth?, allow_pow_only_block?, expect?, max_tip_age?, require_identity? (default true),
     *  allow_demo? (default false), network? }. Gate on `accepted` only. statement is null unless the verdict is 'ok',
     *  or 'demo' with allow_demo (a DEMO chain is cheap to forge). `now` defaults to this device's clock. */
    verifyDeposit({ bundle, ask, now }) {
      const enc = new TextEncoder();
      const parts = [enc.encode(typeof bundle === 'string' ? bundle : JSON.stringify(bundle)), enc.encode(JSON.stringify(ask))];
      const bytes = new Uint8Array(8 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(bytes.buffer);
      view.setBigUint64(0, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      let o = 8;
      for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      return JSON.parse(readString(memory, verify_deposit(ptr, bytes.length)));
    },
  };
}
