#!/usr/bin/env python3
# Build a block's witness fixture (the JSON the host reads via HAZYNC_BLOCK) from a LOCAL archive node,
# instead of public explorers. Same output schema as fetch_block.py — this is a drop-in replacement for
# anyone who has a full node.
#
#   usage: python3 fetch_block_rpc.py <height> <out.json>
#          python3 fetch_block_rpc.py range <lo> <hi> <dir> [--window N] [--jobs N] [--after-saves <prep out>]     (HAZYNC_RPC_CLI="bitcoin-cli" or "ssh <host> bitcoin-cli" for RPC via the CLI)
#
# Why: fetch_block.py talks to blockstream/mempool and needs roughly one request per unique funding tx
# (thousands for a modern block). That is rate-limited into hours, and 429s make it unreliable. A full
# node answers the same questions locally in seconds.
#
# Requirements: bitcoind with `txindex=1` and `prune=0`, and Core >= 25 for `getblock` verbosity 3 —
# which returns each input's `prevout` (height / generated / value / scriptPubKey) inline, so the entire
# per-funding-tx lookup loop collapses into a single call.
import json, sys, http.client, base64, pathlib
from decimal import Decimal

# Defaults target a node on this machine. Overridable so the same script can pull fixtures from a
# node reachable over the network (an SSH tunnel, or an RPC port bound to a private interface) without
# a second copy of this logic drifting from the first:
#
#   HAZYNC_RPC_HOST / HAZYNC_RPC_PORT   where bitcoind's RPC listens
#   HAZYNC_RPC_COOKIE                   path to .cookie, or
#   HAZYNC_RPC_AUTH                     literal "user:password" when cookie auth is not available
#
# Read-only either way: this issues getblockhash and getblock and nothing else.
import os
HOST = os.environ.get("HAZYNC_RPC_HOST", "127.0.0.1")
PORT = int(os.environ.get("HAZYNC_RPC_PORT", "8332"))
COOKIE = pathlib.Path(os.environ.get("HAZYNC_RPC_COOKIE",
                                     str(pathlib.Path.home() / ".bitcoin" / ".cookie")))

def _auth():
    literal = os.environ.get("HAZYNC_RPC_AUTH")
    if literal:
        return base64.b64encode(literal.strip().encode()).decode()
    if not COOKIE.exists():
        raise SystemExit(
            f"no RPC credentials: {COOKIE} does not exist and HAZYNC_RPC_AUTH is unset.\n"
            f"Set HAZYNC_RPC_COOKIE to the node's cookie path, or HAZYNC_RPC_AUTH to 'user:pass'.")
    return base64.b64encode(COOKIE.read_text().strip().encode()).decode()

CLI = os.environ.get("HAZYNC_RPC_CLI")   # e.g. "bitcoin-cli" or "ssh <host> bitcoin-cli": RPC through that command

def rpc(method, params=None):
    if CLI:
        import subprocess, shlex
        args = shlex.split(CLI) + [method] + [p if isinstance(p, str) else json.dumps(p) for p in (params or [])]
        r = subprocess.run(args, capture_output=True, text=True)
        if r.returncode != 0: raise SystemExit(f"{CLI} {method} failed: {r.stderr.strip()}")
        out = r.stdout.strip()
        # bitcoin-cli prints a string result bare, an object or number as JSON
        try: return json.loads(out, parse_float=Decimal)
        except ValueError: return out
    auth = _auth()
    body = json.dumps({"jsonrpc": "2.0", "id": "h", "method": method, "params": params or []})
    c = http.client.HTTPConnection(HOST, PORT, timeout=600)
    c.request("POST", "/", body, {"Authorization": "Basic " + auth, "Content-Type": "application/json"})
    r = c.getresponse().read()
    c.close()
    # Decimal, not float: values arrive in BTC and must convert to satoshis EXACTLY. json's default
    # float parse silently rounds (e.g. 0.1+0.2), which would corrupt the amounts the guest checks.
    d = json.loads(r, parse_float=Decimal)
    if d.get("error"): raise SystemExit(f"RPC {method} failed: {d['error']}")
    return d["result"]

def sats(btc):
    return int((Decimal(btc) * 100_000_000).to_integral_value())

HDR = {}    # header by hash, across blocks of a range
def header(bh):
    if bh not in HDR: HDR[bh] = rpc("getblockheader", [bh])
    return HDR[bh]

MTP_AT = {}  # median time past by height, across blocks of a range

def build(height):
    h = rpc("getblockhash", [height])
    b = rpc("getblock", [h, 3])

    # RETARGET SUPPORT (hazync#83). The guest computes the expected nBits at every height where
    # height % 2016 == 0, using the timestamp of the first block of the PREVIOUS epoch. A fixture that
    # does not carry it forces the host to fabricate one, and the retarget check then compares against
    # a target derived from a made-up timestamp — which is exactly how block 481824 came back
    # `block_valid=true retarget_ok=false`: the block was fine, the fixture could not express it.
    #
    # This is not an edge case to skip. BIP9 soft forks activate ON retarget boundaries by design, so
    # the activation heights that most need a fixture (CSV 419328, segwit 481824) are all retarget
    # blocks. Emitted for every height, since it costs one extra call and a non-retarget block simply
    # carries it unused.
    epoch_first = ((height - 1) // 2016) * 2016 if height > 0 else 0
    eb = rpc("getblock", [rpc("getblockhash", [epoch_first]), 1])
    prev = rpc("getblock", [b["previousblockhash"], 1]) if height > 0 else None
    txs = b["tx"]
    print(f"block {height} ({h[:16]}..): {len(txs)} txs", flush=True)

    # coin_mtp = MTP(coin_height-1) — the median-time-past of the block BEFORE the coin's block. This is
    # what the prover's build path commits and what the guest's BIP68 time check consumes (Core's
    # nCoinTime = GetMedianTimePast(coinHeight-1)). Using the coin block's own nTime is WRONG: nTime >=
    # MTP(h-1), so it over-shoots and false-rejects boundary-tight BIP68 time locks. Cached per height,
    # since many funding txs share a creation block.
    mtp_at = MTP_AT
    def block_mtp(hh):
        if hh < 0: return 0
        if hh not in mtp_at:
            mtp_at[hh] = header(rpc("getblockhash", [hh]))["mediantime"]
        return mtp_at[hh]

    # Previous-11 block timestamps (h-11..h-1), oldest first; their median = MTP(h-1), the spend block's
    # BIP68-time / BIP113 window. Fewer than 11 near genesis is fine (Core uses min(11, height)).
    recent_times, ph = [], b.get("previousblockhash")
    for _ in range(11):
        if not ph: break
        pj = header(ph)
        recent_times.append(pj["time"])
        ph = pj.get("previousblockhash")
    recent_times.reverse()

    out = {"height": b["height"], "version": b["version"], "time": b["time"],
           # bitcoind returns nBits as a hex STRING ("18009645"); the fixture schema (and esplora) use
           # the integer. Convert, or the guest sees a wrong difficulty target.
           "bits": int(b["bits"], 16), "nonce": b["nonce"], "prev": b["previousblockhash"],
           "merkle": b["merkleroot"], "coinbase_hex": txs[0]["hex"],
           "recent_times": recent_times,
           # #83: the in-boundary retarget inputs, so a block at a retarget height can be validated
           # standalone. `epoch_start` is the timestamp of the first block of the PREVIOUS epoch —
           # what Core's CalculateNextWorkRequired takes as nFirstBlockTime. `prev_time`/`prev_bits`
           # are the previous block's, not this one's minus a guess.
           "epoch_start": eb["time"],
           "prev_time": prev["time"] if prev else b["time"],
           "prev_bits": int(prev["bits"], 16) if prev else int(b["bits"], 16),
           "txs": []}

    n_meta = 0
    for idx in range(1, len(txs)):
        t = txs[idx]
        prevs = []
        for v in t["vin"]:
            p = v.get("prevout")
            if p is None:
                raise SystemExit(f"tx {idx} input has no prevout — is this node pruned, or Core < 25?")
            ch = p["height"]
            prevs.append({"value": sats(p["value"]), "spk": p["scriptPubKey"]["hex"],
                          "coin_height": ch,
                          "coin_is_coinbase": 1 if p.get("generated") else 0,
                          "coin_mtp": block_mtp(ch - 1)})
            n_meta += 1
        out["txs"].append({"raw": t["hex"], "prevouts": prevs})

    return out, n_meta

def write(out, path):
    tmp = f"{path}.tmp"
    with open(tmp, "w") as f: json.dump(out, f)
    os.replace(tmp, path)   # a consumer never reads half a file

def main():
    # range <lo> <hi> <dir> [--window N]: every block lo..hi into <dir>/block_<h>.json (existing files kept), waiting
    # while N block files are already waiting to be consumed (libzync-prep with HAZYNC_PREP_CONSUME=1 deletes each
    # after use), so a whole-chain prep never needs the whole chain's JSON on disk.
    if sys.argv[1] == "range":
        lo, hi, d = int(sys.argv[2]), int(sys.argv[3]), pathlib.Path(sys.argv[4])
        window = int(sys.argv[sys.argv.index("--window") + 1]) if "--window" in sys.argv else 0
        # --jobs N: N processes, process k fetching heights lo+k, lo+k+N, ... (RPC and JSON decoding are the cost)
        jobs = int(sys.argv[sys.argv.index("--jobs") + 1]) if "--jobs" in sys.argv else 1
        # --after-saves <prep out dir>: start after prep's newest save point (state_<h>.smt), the height a restarted
        # prep resumes from; the files below it were consumed and deleted, and must not be fetched again
        if "--after-saves" in sys.argv:
            od = pathlib.Path(sys.argv[sys.argv.index("--after-saves") + 1])
            # a COMPLETE save point only (as libzync-prep-run.sh): a stop during a save leaves a newer partial one
            # (state_<h>.smt without its .coinlog / .trie), and prep then resumes from the one before it
            saves = [int(q.name[6:-4]) for q in od.glob("state_*.smt") if q.name[6:-4].isdigit()
                     and all((od / f"{q.name[:-4]}.{x}").exists() for x in ("json", "coinlog", "trie"))] if od.exists() else []
            if saves and max(saves) + 1 > lo:
                lo = max(saves) + 1
                print(f"resuming after prep's save point {lo - 1}", flush=True)
        d.mkdir(parents=True, exist_ok=True)
        # ⛔ prep with HAZYNC_PREP_CONSUME=1 deletes files only at its SAVE POINTS: the window must be larger than
        # HAZYNC_PREP_SAVE_EVERY, or the producer waits for a save that waits for the producer
        def worker(k):
            import time
            for height in range(lo + k, hi + 1, jobs):
                p = d / f"block_{height}.json"
                if p.exists(): continue
                # files waiting below this height: only those count, so the block prep needs next is never held back
                while window and sum(1 for q in d.glob("block_*.json") if q.name[6:-5].isdigit() and int(q.name[6:-5]) < height) >= window:
                    time.sleep(1)
                out, n_meta = build(height)
                write(out, p)
                print(f"block {height}: {len(out['txs'])} txs, {n_meta} prevouts", flush=True)
        if jobs == 1:
            worker(0)
            return
        import multiprocessing
        ps = [multiprocessing.Process(target=worker, args=(k,)) for k in range(jobs)]
        for x in ps: x.start()
        for x in ps: x.join()
        bad = [x.exitcode for x in ps if x.exitcode != 0]
        if bad: raise SystemExit(f"{len(bad)} fetch worker(s) failed: exit codes {bad}")
        return
    height, out_path = int(sys.argv[1]), sys.argv[2]
    out, n_meta = build(height)
    write(out, out_path)
    print(f"saved {out_path}: {len(out['txs'])} txs, {n_meta} prevouts", flush=True)

if __name__ == "__main__":
    main()
