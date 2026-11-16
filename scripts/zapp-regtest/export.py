#!/usr/bin/env python3
"""export.py <chain dir> <out dir>: replay the committed chain into a fresh Bitcoin Core regtest node and write, from the
node alone, the block files the libzync host reads and the oracle snapshots the zAPP tests compare against.

Needs only a bitcoind (BITCOIND, Core 28.0 as the CI rig) and Python 3: no test_framework, no libzync code. The block
files are written by the repo's own prover/fetch_block_rpc.py (the deployed format), pointed at this node.

Reads  <chain dir>/raw.txt (gen_chain.py), <chain dir>/scenarios.json (the harness scripts to scan for).
Writes <out>/blocks/{main,a,b}/block_<h>.json   main = 1..110, a = 111..112, b = 111..113 (prover format)
       <out>/oracle/snap_<label>.json            at 101, 104, 105, 110, a112, b113: the tip, gettxoutsetinfo
                                                 (hash_serialized_3 and muhash), gettxout for every output the chain
                                                 created, scantxoutset for every harness script, dumptxoutset's summary
       <out>/oracle/utxo_<label>.dat             the dumptxoutset file itself
       <out>/oracle/blocks_<branch>.json         per block: header, txids in order, every input's spend, and
                                                 gettxoutproof for every transaction (checked by verifytxoutproof)
Checks on the way: every block is accepted and becomes the tip; the reorg to b happens at b 113; invalidateblock b 111
returns the node to a 112 with a snapshot equal to the first a 112 one, and reconsiderblock to b 113 likewise.

Env: BITCOIND, RT_PORT (RPC port, default 18998; P2P port RT_PORT+1), FETCH (prover/fetch_block_rpc.py).
"""
import base64, hashlib, http.client, json, os, shutil, subprocess, sys, tempfile, time
from decimal import Decimal

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = os.environ.get("BITCOIND", os.path.expanduser("~/libzync-gpu/rig/dl/bitcoin-28.0/bin/bitcoind"))
PORT = int(os.environ.get("RT_PORT", "18998"))
FETCH = os.environ.get("FETCH", os.path.join(HERE, "../../prover/fetch_block_rpc.py"))
SNAPS = {101: "101", 104: "104", 105: "105", 110: "110"}
DATA = None


def rpc(method, *params):
    auth = base64.b64encode(open(f"{DATA}/regtest/.cookie").read().strip().encode()).decode()
    c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=600)
    c.request("POST", "/", json.dumps({"jsonrpc": "1.0", "id": 0, "method": method, "params": list(params)}),
              {"Authorization": "Basic " + auth, "Content-Type": "application/json"})
    d = json.loads(c.getresponse().read(), parse_float=Decimal)
    c.close()
    if d.get("error"):
        raise RuntimeError(f"{method}: {d['error']}")
    return d["result"]


def sats(btc):
    return int((Decimal(btc) * 100_000_000).to_integral_value())


def j(x):
    """JSON-safe: Decimal -> int satoshis is done by the callers; any Decimal left is written as a string."""
    if isinstance(x, Decimal):
        return str(x)
    if isinstance(x, dict):
        return {k: j(v) for k, v in x.items()}
    if isinstance(x, list):
        return [j(v) for v in x]
    return x


def wait_indexes():
    tip = rpc("getblockcount")
    for _ in range(600):
        ix = rpc("getindexinfo")
        if all(v["synced"] and v["best_block_height"] == tip for v in ix.values()) and len(ix) == 2:
            return
        time.sleep(0.1)
    raise RuntimeError(f"indexes not synced to {tip}: {ix}")


def header(bh):
    hd = rpc("getblockheader", bh)
    hd.pop("confirmations", None); hd.pop("nextblockhash", None)
    return j(hd)


def outputs_of_chain():
    """Every output every block of the active chain created, in chain order: [(txid, n)]."""
    outs = []
    for hh in range(1, rpc("getblockcount") + 1):
        for t in rpc("getblock", rpc("getblockhash", hh), 2)["tx"]:
            outs += [(t["txid"], o["n"]) for o in t["vout"]]
    return outs


def snapshot(label, scripts, odir):
    wait_indexes()
    tip = rpc("getblockcount"); bh = rpc("getbestblockhash")
    s = {"label": label, "height": tip, "hash": bh, "header": header(bh)}
    for kind in ("hash_serialized_3", "muhash"):
        g = rpc("gettxoutsetinfo", kind)
        g.pop("disk_size", None)
        g["total_amount_sat"] = sats(g.pop("total_amount"))
        s["gettxoutsetinfo_" + kind] = j(g)
    utxo = {}
    for txid, n in outputs_of_chain():
        # Core: null when spent, never a coin (unspendable) or not on this chain. include_mempool=false: after a reorg
        # the disconnected branch's transactions are back in the mempool, and the default (true) then reports a coin
        # unspent on the active chain as spent (found here: X read null at b 113 because a's spend of it was pooled)
        o = rpc("gettxout", txid, n, False)
        utxo[f"{txid}:{n}"] = None if o is None else {
            "value_sat": sats(o["value"]), "spk": o["scriptPubKey"]["hex"], "coinbase": o["coinbase"],
            "height": tip - o["confirmations"] + 1}
    s["gettxout"] = utxo
    scan = {}
    for name, spk in scripts:
        r = rpc("scantxoutset", "start", [f"raw({spk})"])
        assert r["success"] and r["height"] == tip, r
        scan[name] = {"spk": spk, "total_amount_sat": sats(r["total_amount"]), "n": len(r["unspents"]),
                      "unspents": sorted(({"txid": u["txid"], "vout": u["vout"], "amount_sat": sats(u["amount"]),
                                           "height": u["height"], "coinbase": u["coinbase"]} for u in r["unspents"]),
                                         key=lambda u: (u["height"], u["txid"], u["vout"]))}
    s["scantxoutset"] = scan
    p = f"{odir}/utxo_{label}.dat"
    if os.path.exists(p):
        os.remove(p)
    d = rpc("dumptxoutset", p)
    d.pop("path")
    d["file_sha256"] = hashlib.sha256(open(p, "rb").read()).hexdigest()
    d["file_bytes"] = os.path.getsize(p)
    s["dumptxoutset"] = j(d)
    return s


def diff(x, y, at=""):
    """The paths where two snapshots differ (for the round-trip check's message)."""
    if isinstance(x, dict) and isinstance(y, dict):
        return [d for k in sorted(set(x) | set(y)) for d in diff(x.get(k), y.get(k), f"{at}.{k}")]
    return [] if x == y else [f"{at}: {str(x)[:80]} != {str(y)[:80]}"]


def write(path, obj):
    with open(path, "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True)
        f.write("\n")


def blocks_oracle(lo, hi):
    """Per block of the active chain lo..hi: header, txids, spends and a Merkle proof per transaction."""
    out = {}
    for hh in range(lo, hi + 1):
        bh = rpc("getblockhash", hh)
        b = rpc("getblock", bh, 2)
        spends, proofs = [], {}
        for t in b["tx"]:
            for i, x in enumerate(t["vin"]):
                if "coinbase" not in x:
                    spends.append({"prev": f"{x['txid']}:{x['vout']}", "txid": t["txid"], "vin": i})
            pr = rpc("gettxoutproof", [t["txid"]], bh)
            assert rpc("verifytxoutproof", pr) == [t["txid"]], t["txid"]
            proofs[t["txid"]] = pr
        out[str(hh)] = {"hash": bh, "header": header(bh), "txids": [t["txid"] for t in b["tx"]],
                        "sizes": [t["size"] for t in b["tx"]], "spends": spends, "gettxoutproof": proofs}
    return out


def fetch(lo, hi, d):
    os.makedirs(d, exist_ok=True)
    env = dict(os.environ, HAZYNC_RPC_PORT=str(PORT), HAZYNC_RPC_COOKIE=f"{DATA}/regtest/.cookie")
    r = subprocess.run([sys.executable, FETCH, "range", str(lo), str(hi), d], env=env, capture_output=True, text=True)
    assert r.returncode == 0, r.stdout[-500:] + r.stderr[-1500:]


def main():
    global DATA
    cdir, out = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    raw = [l.split() for l in open(f"{cdir}/raw.txt")]
    sc = json.load(open(f"{cdir}/scenarios.json"))
    k = sc["keys"]
    scripts = [(f"MINER.{t}", v["scripts"][t]["spk"]) for t, v in k["MINER"].items()] + \
              [(f"ALICE.{t}", v["scripts"][t]["spk"]) for t, v in k["ALICE"].items()] + \
              [("DEPOSIT.p2wpkh", k["DEPOSIT"]["scripts"]["p2wpkh"]["spk"]), ("CHANGE.p2wpkh", k["CHANGE"]["scripts"]["p2wpkh"]["spk"]),
               ("ANYONE", k["ANYONE"]["spk"]), ("TX64_OUTPUT", "51515151")]
    for sub in ("blocks", "oracle"):
        shutil.rmtree(f"{out}/{sub}", ignore_errors=True)
    os.makedirs(f"{out}/oracle")
    od = f"{out}/oracle"
    DATA = tempfile.mkdtemp(prefix="zapp-regtest-export-")
    node = subprocess.Popen([os.path.abspath(BIN), "-regtest", f"-datadir={DATA}", f"-rpcport={PORT}", f"-port={PORT + 1}", "-listen=0",
                             "-testactivationheight=bip34@110", "-txindex=1", "-coinstatsindex=1"], stdout=subprocess.DEVNULL)
    try:
        for _ in range(200):
            try:
                rpc("getblockcount"); break
            except (OSError, RuntimeError):
                time.sleep(0.2)
        print("core:", rpc("getnetworkinfo")["subversion"], flush=True)

        for br, hh, bh, hx in [r for r in raw if r[0] in ("main", "a")]:
            got = rpc("submitblock", hx)
            assert got is None, f"{br} {hh}: Core refused: {got}"
            assert rpc("getbestblockhash") == bh, f"{br} {hh} is not the tip"
            if br == "main" and int(hh) in SNAPS:
                write(f"{od}/snap_{SNAPS[int(hh)]}.json", snapshot(SNAPS[int(hh)], scripts, od))
        a112 = snapshot("a112", scripts, od)
        write(f"{od}/snap_a112.json", a112)
        fetch(1, 112, f"{out}/blocks/main")
        os.makedirs(f"{out}/blocks/a")
        for hh in (111, 112):
            os.replace(f"{out}/blocks/main/block_{hh}.json", f"{out}/blocks/a/block_{hh}.json")
        write(f"{od}/blocks_main.json", blocks_oracle(1, 110))
        write(f"{od}/blocks_a.json", blocks_oracle(111, 112))
        bb = [r for r in raw if r[0] == "b"]
        for i, (br, hh, bh, hx) in enumerate(bb):
            got = rpc("submitblock", hx)
            # a side branch with no more work than a is stored but not connected: Core says "inconclusive"
            want = None if i == len(bb) - 1 else "inconclusive"
            assert got == want, f"b {hh}: Core said {got}, expected {want}"
        btip = [r for r in raw if r[0] == "b"][-1][2]
        assert rpc("getbestblockhash") == btip, "the reorg to b did not happen"
        b113 = snapshot("b113", scripts, od)
        write(f"{od}/snap_b113.json", b113)
        fetch(111, 113, f"{out}/blocks/b")
        write(f"{od}/blocks_b.json", blocks_oracle(111, 113))
        tips = rpc("getchaintips")
        write(f"{od}/chaintips.json", j(sorted(tips, key=lambda t: t["height"])))
        # back and forth: invalidate b 111 -> a 112 again; reconsider -> b 113 again; the snapshots must not move
        b111 = [r for r in raw if r[0] == "b"][0][2]
        rpc("invalidateblock", b111)
        assert rpc("getbestblockhash") == [r for r in raw if r[0] == "a"][-1][2], "invalidateblock b 111 did not return to a 112"
        again = snapshot("a112", scripts, od)
        assert again == a112, "a 112 after the round trip differs: " + str(diff(a112, again))
        rpc("reconsiderblock", b111)
        assert rpc("getbestblockhash") == btip, "reconsiderblock b 111 did not return to b 113"
        again = snapshot("b113", scripts, od)
        assert again == b113, "b 113 after the round trip differs: " + str(diff(b113, again))
        n = sum(len(os.listdir(f"{out}/blocks/{b}")) for b in ("main", "a", "b"))
        print(f"EXPORT_OK {n} block files, snapshots {sorted(list(SNAPS.values()) + ['a112', 'b113'])}, reorg round trip equal")
    finally:
        try:
            rpc("stop")
        except Exception:
            pass
        node.wait(timeout=60)
        shutil.rmtree(DATA, ignore_errors=True)


main()
