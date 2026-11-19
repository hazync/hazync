// Glue for the header-chain zyncAPP's WASM module (ROADMAP K1, N3), a zAPP module of its own (docs/libzync/ZYNCAPPS.md
// §9), not the core verifier's hazync-verify.js (until 15.11.2026 verifyHeaders was there). Hand-written and short on
// purpose, as the core's: no wasm-bindgen, nothing here touches the network.
//
//   import { loadHeaders } from './zapp-headers.js';
//   const v = await loadHeaders(fetch('./zapp-headers.wasm'));
//   const h = v.verifyHeaders({ receipts: [rcpt], expect: '<height>:<hash>' });
//   h.verdict, h.height, h.hash, h.chainwork, h.mmr_root
//   v.verifyHeaders({ receipts, include: { height, hash, path } })   // and block <hash> is at <height> on that chain
//   (path: the lines `zyncapp headers-mmr-path` prints, or the path service's /path answer)
//   v.verifyHeaders({ receipts, include: { height, hash, path, txid, tx } })   // and transaction <txid> is in it
//   (txid: the transaction YOU ask about, never taken from the answer; tx: the path service's /tx answer
//   { pos, header, raw, branch, coinbase, coinbase_branch }: the raw transaction must hash to txid, the raw coinbase
//   and its branch pin the tree's depth. The answer's own txid is not read: an answer about another transaction fails.)
//   READ included WITH THE VERDICT:
//   h.included       true only when h.verdict === 'ok' and the block (and transaction) is on the chain; null if not asked
//   h.included_demo  true when h.verdict === 'demo' (a valid chain BELOW Core's minimum work, which anyone can mine
//                    on a cheap difficulty-1 fork, or one whose tip was not compared with a source you trust: shown,
//                    not a claim about the network's chain) and the checks passed
//   'ok' needs BOTH Core's minimum work and expect matched (h.identity_checked): without an expect, even the whole
//   mainnet header chain is 'demo' (since 16.11.2026; give the tip from a source you trust, e.g. an explorer)
//   h.inclusion      { height, hash, txid } of what was shown, or null (non-null for 'ok' and 'demo' alike)
//   ASKING ABOUT A TRANSACTION, also check h.inclusion.txid === the txid you asked about (and height / hash): included
//   says something was shown, inclusion says WHAT. Leave txid and tx out (not null: refused) to ask about the block alone.

function readString(memory, ptr) {
  const view = new DataView(memory.buffer);
  const len = view.getUint32(ptr, true);
  return new TextDecoder().decode(new Uint8Array(memory.buffer, ptr + 4, len));
}

export async function loadHeaders(source) {
  const resolved = await source;
  const { instance } =
    typeof Response !== 'undefined' && resolved instanceof Response
      ? await WebAssembly.instantiateStreaming(resolved, {})
      : await WebAssembly.instantiate(resolved instanceof Uint8Array ? resolved : new Uint8Array(resolved), {});
  const { alloc, verify_headers, headers_image_id, memory } = instance.exports;
  return {
    /** The header-chain zyncAPP: receipts = [first from genesis, then each run chained to the one before];
     *  expect ('<height>:<hash>') is the tip from a source you trust. `now` defaults to this device's clock.
     *  include ({ height, hash, path, txid?, tx? }): also check that this block (and transaction txid, the one asked
     *  about, by the /tx answer tx) is on the proven chain. included is true only for verdict 'ok'; included_demo
     *  for 'demo'; inclusion names what was shown: asking about a transaction, check inclusion.txid is the txid asked
     *  about. txid / tx null are refused (leave both out for the block alone). */
    verifyHeaders({ receipts, now, imageHex, expect, include }) {
      const enc = new TextEncoder();
      const parts = [...receipts, enc.encode(imageHex || ''), enc.encode(expect || ''),
                     enc.encode(include ? JSON.stringify(include) : '')];
      const bytes = new Uint8Array(12 + parts.reduce((n, p) => n + 4 + p.length, 0));
      const view = new DataView(bytes.buffer);
      view.setBigUint64(0, BigInt(Math.floor(now ?? Date.now() / 1000)), true);
      view.setUint32(8, receipts.length, true);
      let o = 12;
      for (const p of parts) { view.setUint32(o, p.length, true); bytes.set(p, o + 4); o += 4 + p.length; }
      const ptr = alloc(bytes.length);
      new Uint8Array(memory.buffer, ptr, bytes.length).set(bytes);
      return JSON.parse(readString(memory, verify_headers(ptr, bytes.length)));
    },
    /** The built-in header-chain image id. */
    headersImageId() { return readString(memory, headers_image_id()); },
  };
}
