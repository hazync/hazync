#!/usr/bin/env python3
"""oracle_node.py <a|b> [-regtest] <rpc> [args...]: a stand-in `bitcoin-cli -regtest` for the zAPP tests, answering from
the committed chain (blocks/) and Core's oracle (oracle/) alone, as Core's node answers on that branch: `a` = main 1..110
+ a 111..112 (after `invalidateblock` b 111), `b` = main + b 111'..113' (the active chain after the reorg). A zAPP CLI
takes it as `--bitcoin-cli "python3 scripts/zapp-regtest/oracle_node.py b -regtest"`; nothing here comes from a zAPP.

RPCs: getblockcount, getbestblockhash, getblockhash <h>, getblockheader <hash> (Core's JSON, chainwork included),
getblock <hash> 0 (the raw block, rebuilt from the block file and checked against its hash), gettxout <txid> <n> false
(only at the branch's tip, from the snapshot; without include_mempool=false it is REFUSED: after the reorg the default
counts the disconnected branch's pooled spends, README "Oracle"). Harness only: `headers <file>` writes the 80-byte
headers 0..tip (each hash checked against Core's). Errors: Core's message on stderr, exit 1 (bitcoin-cli's code)."""
import hashlib, json, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
GENESIS = ("0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
           # regtest genesis: version 1, no prev, its merkle root, time, bits, nonce 2
           bytes.fromhex("01000000" + "00" * 32 + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
                         + "dae5494d" + "ffff7f20" + "02000000"))


def die(msg):
    print(f"error code: -8\nerror message:\n{msg}", file=sys.stderr)
    sys.exit(1)


def dsha(b):
    return hashlib.sha256(hashlib.sha256(b).digest()).digest()


def load(chain):
    main = json.load(open(f"{HERE}/oracle/blocks_main.json"))
    br = json.load(open(f"{HERE}/oracle/blocks_{chain}.json"))
    by_h = {0: {"hash": GENESIS[0]}}
    for src, name in ((main, "main"), (br, chain)):
        for h, v in src.items():
            by_h[int(h)] = dict(v, dir=name)
    return by_h, json.load(open(f"{HERE}/oracle/snap_{'a112' if chain == 'a' else 'b113'}.json"))


def raw_header(by_h, h):
    if h == 0:
        return GENESIS[1]
    f = json.load(open(f"{HERE}/blocks/{by_h[h]['dir']}/block_{h}.json"))
    hd = (int(f["version"]).to_bytes(4, "little") + bytes.fromhex(f["prev"])[::-1] + bytes.fromhex(f["merkle"])[::-1]
          + int(f["time"]).to_bytes(4, "little") + int(f["bits"]).to_bytes(4, "little") + int(f["nonce"]).to_bytes(4, "little"))
    assert dsha(hd)[::-1].hex() == by_h[h]["hash"], f"block file {h} does not hash to Core's block {h}"
    return hd


def varint(n):
    return bytes([n]) if n < 0xfd else b"\xfd" + n.to_bytes(2, "little") if n <= 0xffff else b"\xfe" + n.to_bytes(4, "little")


def main():
    a = sys.argv[1:]
    if not a or a[0] not in ("a", "b"):
        die(__doc__)
    chain, a = a[0], [x for x in a[1:] if x != "-regtest"]
    by_h, snap = load(chain)
    tip = max(by_h)
    height_of = {v["hash"]: h for h, v in by_h.items()}
    rpc, args = (a[0], a[1:]) if a else ("", [])
    if rpc == "getblockcount":
        print(tip)
    elif rpc == "getbestblockhash":
        print(by_h[tip]["hash"])
    elif rpc == "getblockhash":
        h = int(args[0])
        if h not in by_h:
            die("Block height out of range")
        print(by_h[h]["hash"])
    elif rpc == "getblockheader":
        h = height_of.get(args[0].lower())
        if h is None:
            die("Block not found")
        if h == 0:
            print(json.dumps({"hash": GENESIS[0], "height": 0, "chainwork": "%064x" % 2}))
        else:
            print(json.dumps(by_h[h]["header"], indent=2))
    elif rpc == "getblock" and args[1:] == ["0"]:
        h = height_of.get(args[0].lower())
        if h is None or h == 0:
            die("Block not found")
        f = json.load(open(f"{HERE}/blocks/{by_h[h]['dir']}/block_{h}.json"))
        txs = [f["coinbase_hex"]] + [t["raw"] for t in f["txs"]]
        print((raw_header(by_h, h) + varint(len(txs)) + b"".join(bytes.fromhex(t) for t in txs)).hex())
    elif rpc == "gettxout":
        if args[2:3] != ["false"]:
            die("oracle_node: gettxout must be asked with include_mempool=false (the reorg trap, scripts/zapp-regtest/README.md)")
        c = snap["gettxout"].get(f"{args[0].lower()}:{int(args[1])}")
        if c is not None:
            print(json.dumps({"bestblock": by_h[tip]["hash"], "value_sat": c["value_sat"], "coinbase": c["coinbase"],
                              "scriptPubKey": {"hex": c["spk"]}, "height": c["height"]}))
    elif rpc == "headers":
        with open(args[0], "wb") as o:
            for h in range(tip + 1):
                o.write(raw_header(by_h, h))
        print(f"{tip + 1} headers 0..{tip} of branch {chain}, tip {by_h[tip]['hash']}", file=sys.stderr)
    else:
        die(f"oracle_node: no such RPC here: {' '.join(a)}")


main()
