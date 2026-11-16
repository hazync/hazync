#!/usr/bin/env python3
# Build a block's witness fixture (the JSON the host reads via HAZYNC_BLOCK) from a LOCAL archive node,
# instead of public explorers. Same output schema as fetch_block.py — this is a drop-in replacement for
# anyone who has a full node.
#
#   usage: python3 fetch_block_rpc.py <height> <out.json>
#          python3 fetch_block_rpc.py range <lo> <hi> <dir> [--window N] [--jobs N] [--after-saves <prep out>]     (HAZYNC_RPC_CLI="bitcoin-cli" or "ssh <host> bitcoin-cli" for RPC via the CLI)
#          python3 fetch_block_rpc.py follow <lo> <dir> --after-saves <prep out> [--hi H] [--window N] [--jobs N]
#          python3 fetch_block_rpc.py fork-point <prep out> [--deep]
#
# With --after-saves, `range` and `follow` also watch the chain prep is on (docs/libzync/REORG.md §4): a fork below prep's
# tip writes <dir>/REORG (or REORG_TOO_DEEP) and exits 4; a fork above it replaces the waiting block files.
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

# ── Reorg detection (docs/libzync/REORG.md §4; phase P2: detect and stop, no rewind yet) ─────────────────────────────
#
# ONE comparison catches a reorg of any depth: if the node's active chain has prep's tip hash at prep's tip height, every
# block below it is on the active chain too (headers chain by hash). If not, the fork F is the highest height at or
# below prep's tip where prep's state_<h>.json and the node agree. Prep's states are the source of truth: they say
# which chain the board proves.
#
#   a fork above prep's tip      the waiting block files above it are deleted and fetched again; nothing else changes
#   a fork below prep's tip      <dir>/REORG (F, depth, the orphaned and the new hashes), exit 4. P2 stops for EVERY
#                                such fork; P3 will rewind those within W by itself. Depth <= W is marked "auto" (P3
#                                handles it), W < depth <= J "manual" (one command, the undo records reach it)
#   no agreement within J        <dir>/REORG_TOO_DEEP, exit 4 (the undo records cannot reach F: REORG.md §11)
#
# Exit 4 is the units' RestartPreventExitStatus: the unit fails and its OnFailure alert fires. While a marker is in
# <dir>, this fetcher and libzync-prep refuse to start (exit 4 again); a person removes it (REORG.md §11).
EXIT_REORG = 4
MARKERS = ("REORG", "REORG_TOO_DEEP")
POLL_S = float(os.environ.get("HAZYNC_FETCH_POLL_S", "5"))
W_AUTO = int(os.environ.get("HAZYNC_REORG_AUTO_DEPTH", "288"))      # W: the automatic window (P3)
J_KEEP = int(os.environ.get("HAZYNC_PREP_UNDO_KEEP", "2016"))       # J: prep's undo records (reorg.rs)

import re, struct, hashlib, time
_HEAD = {k: re.compile(rf'"{k}":\s*(-?\d+)') for k in ("height", "version", "time", "bits", "nonce")}
_HEAD.update({k: re.compile(rf'"{k}":\s*"([0-9a-f]{{64}})"') for k in ("prev", "merkle")})

def header_hash(f):
    """A block's hash (display hex) from its header fields as a block file carries them."""
    hdr = (struct.pack("<I", int(f["version"]) & 0xffffffff) + bytes.fromhex(f["prev"])[::-1] + bytes.fromhex(f["merkle"])[::-1]
           + struct.pack("<III", int(f["time"]), int(f["bits"]), int(f["nonce"])))
    return hashlib.sha256(hashlib.sha256(hdr).digest()).digest()[::-1].hex()

def file_block(p):
    """(height, hash) of a block file, or None if it cannot be read. The header fields come before the transactions in
    every writer's key order, so the first 4 KB are read (a mainnet block file is MBs); else the whole file."""
    try:
        with open(p, "rb") as fh: head = fh.read(4096).decode("utf-8", "replace")
        f = {k: m.group(1) for k, r in _HEAD.items() for m in [r.search(head)] if m}
        if len(f) < len(_HEAD):
            j = json.load(open(p))
            f = {k: j[k] for k in _HEAD}
        return int(f["height"]), header_hash(f)
    except (OSError, ValueError, KeyError, TypeError):
        return None

def state_hash(od, h):
    """The block hash (display hex) of prep's state_<h>.json, or None (absent, or a write in progress: prep writes the
    states between save points in place)."""
    try:
        t = json.load(open(od / f"state_{h}.json"))["tip_hash"]
        return bytes(t)[::-1].hex() if len(t) == 32 else None
    except (OSError, ValueError, KeyError, TypeError):
        return None

def complete_saves(od):
    """prep's COMPLETE save points (as libzync-prep-run.sh): a stop during a save leaves a newer partial one."""
    return [int(q.name[6:-4]) for q in od.glob("state_*.smt") if q.name[6:-4].isdigit()
            and all((od / f"{q.name[:-4]}.{x}").exists() for x in ("json", "coinlog", "trie"))] if od.exists() else []

class CannotCheck(Exception):
    pass

class PrepTip:
    """prep's tip. From <out>/PREP_TIP.json, which prep writes after every block and at its start (prover/host/src/
    reorg.rs write_tip; review of P2, M2): the height its STORES are at, with J and the lowest height its undo records
    reach. Without that file (a host from before it), the newest state_<h>.json that reads, probing upward by stat
    (never listing the out dir: 632,587 files on the box) and, when the newest do not read, down to the newest
    complete save point, whose state is synced (review M1: three unreadable files used to read as "no prep at all",
    which turned detection off). States that exist but none of which read: CannotCheck."""
    def __init__(self, od):
        self.od, self.h = od, max(complete_saves(od), default=0)
        self.keep, self.undo_from, self.source = None, None, None
    def get(self):
        try:
            t = json.loads((self.od / "PREP_TIP.json").read_text())
            h, x = int(t["height"]), str(t["hash"])
            if len(x) == 64:
                self.h, self.keep, self.undo_from, self.source = h, int(t["undo_keep"]), int(t["undo_from"]), "PREP_TIP.json"
                return h, x
        except (OSError, ValueError, KeyError, TypeError):
            pass
        self.keep, self.undo_from, self.source = None, None, "state files"
        save = max(complete_saves(self.od), default=0)
        if self.h and not (self.od / f"state_{self.h}.json").exists():
            self.h = save                                              # moved away (a rewind, P3): start again
        while (self.od / f"state_{self.h + 1}.json").exists(): self.h += 1
        for h in range(self.h, min(save, self.h) - 1, -1):            # the newest may be half written
            if h == 0: break
            x = state_hash(self.od, h)
            if x: return h, x
        if self.h == 0 and save == 0: return 0, None                  # no state file at all: a run not yet started
        raise CannotCheck(f"state files up to {self.h} exist but none from there down to the save point {save} reads")

def chain_info():
    i = rpc("getblockchaininfo")
    return int(i["blocks"]), i["bestblockhash"]

def walk_back(od, top, limit):
    """The fork: the highest h in [top - limit, top] where prep's state and the node agree, and the deepest height
    compared. Repeated if the node's best block moves during the walk, so every hash compared is from one chain. A
    state that is missing or does not read is CannotCheck (review M4): skipping it would put the fork deeper than it
    is, on a marker a person then acts on. (Prep syncs its states at every save point, so this means damage.)"""
    while True:
        _, best = chain_info()
        fork, deepest = None, top
        for h in range(top, max(top - limit, 0) - 1, -1):
            deepest = h
            if h == 0: fork = 0; break                                 # genesis: the same network's, always shared
            x = state_hash(od, h)
            if x is None: raise CannotCheck(f"state_{h}.json is missing or does not read, inside the fork walk")
            if x == rpc("getblockhash", [h]): fork = h; break
        if chain_info()[1] == best: return fork, deepest

def write_marker(d, name, body):
    p = d / name
    tmp = d / f".{name}.tmp"
    tmp.write_text(json.dumps(body, indent=1) + "\n")
    os.replace(tmp, p)
    return p

def refuse_if_marked(d):
    for m in MARKERS:
        if (d / m).exists():
            print(f"FETCH_STOPPED_REORG {d / m} is there: a person decides (docs/libzync/REORG.md §11); remove it once "
                  f"prep's stores are on the active chain", flush=True)
            sys.exit(EXIT_REORG)

def check_prep(d, od, tip):
    """The detection check against prep's tip. Returns prep's tip height, or None while the node is behind prep (a node
    still syncing: nothing can be decided, and nothing is fetched). A fork below prep's tip writes the marker and exits.
    Depth is counted from PREP's tip (review L1), J and the records' reach are prep's own when PREP_TIP.json says them
    (review L2): a fork the records do not reach (deeper than J, or below the height the records began at) is
    REORG_TOO_DEEP. Unreadable prep state is a stop too (review M1), never "no reorg"."""
    try:
        P, hp = tip.get()
    except CannotCheck as e:
        print(f"FETCH_STOPPED_CANNOT_CHECK {e}: prep's chain cannot be compared with the node's; a person decides", flush=True)
        sys.exit(EXIT_REORG)
    if P == 0: return 0
    J = tip.keep if tip.keep is not None else J_KEEP
    node_h, best = chain_info()
    at = min(P, node_h)
    if at == P and rpc("getblockhash", [P]) == hp: return P
    if at < P and state_hash(od, at) == rpc("getblockhash", [at]):
        print(f"the node is at {node_h}, below prep's tip {P}, on prep's chain: waiting for it", flush=True)
        return None
    try:
        fork, deepest = walk_back(od, at, max(J - (P - at), 0)) if P - at <= J else (None, at)
    except CannotCheck as e:
        print(f"FETCH_STOPPED_CANNOT_CHECK {e}: the fork cannot be placed; a person decides", flush=True)
        sys.exit(EXIT_REORG)
    if fork is not None and fork >= at:
        return P if at == P else None    # the node moved between the two looks and now agrees: nothing to stop for
    now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    reach = tip.undo_from if tip.undo_from is not None else None
    if fork is None or (reach is not None and fork + 1 < reach):
        why = (f"no agreement with prep's states from {at} down to {deepest} (J={J})" if fork is None else
               f"the fork at {fork} is below the lowest undo record ({reach}): the records cannot undo it")
        p = write_marker(d, "REORG_TOO_DEEP", {"prep_tip": P, "prep_tip_hash": hp, "node_tip": node_h, "node_best": best,
                                               "fork": fork, "depth": None if fork is None else P - fork,
                                               "deepest_compared": deepest, "J": J, "undo_from": reach, "why": why,
                                               "at": now, "next": "fetch_block_rpc.py fork-point <prep out> --deep; REORG.md §11"})
        print(f"FETCH_STOPPED_REORG {why}: {p}", flush=True)
        sys.exit(EXIT_REORG)
    depth = P - fork
    orphaned = [[h, state_hash(od, h) if h < P else hp] for h in range(fork + 1, P + 1)]
    new = [[h, rpc("getblockhash", [h])] for h in range(fork + 1, min(P, node_h) + 1)]
    cls = "auto" if depth <= W_AUTO else "manual"
    p = write_marker(d, "REORG", {"fork": fork, "fork_hash": rpc("getblockhash", [fork]) if fork else None, "prep_tip": P,
                                  "depth": depth, "class": cls, "W": W_AUTO, "J": J, "undo_from": reach, "node_tip": node_h,
                                  "node_best": best, "prep_tip_from": tip.source, "orphaned": orphaned, "new": new, "at": now})
    print(f"FETCH_STOPPED_REORG prep's tip {P} is not on the node's chain: fork at {fork}, depth {depth} ({cls}; "
          f"W={W_AUTO}, J={J}): {p}", flush=True)
    sys.exit(EXIT_REORG)

def scan_waiting(d, above, node_h, seen=None):
    """Deletes every waiting block file above `above` that is not the node's block at its height (or that cannot be
    read). With `seen` (name -> (mtime, size) of files already checked against the chain as it is now), only files new
    or changed since are checked, and the checked ones are added: run every poll, this catches a file fetched while
    the node was briefly on another branch even when its best block is back where it was by the next poll (review
    L4). A full check (seen=None) is made with no fetch worker running, after the best block changed. Returns the
    heights deleted."""
    gone = []
    for q in d.glob("block_*.json"):
        if not q.name[6:-5].isdigit(): continue
        h = int(q.name[6:-5])
        if h <= above: continue
        try: stv = q.stat(); sig = (stv.st_mtime_ns, stv.st_size)
        except OSError: continue
        if seen is not None and seen.get(q.name) == sig: continue
        fb = file_block(q)
        if fb is None or fb[0] != h or h > node_h or fb[1] != rpc("getblockhash", [h]):
            q.unlink(missing_ok=True)
            gone.append(h)
            if seen is not None: seen.pop(q.name, None)
        elif seen is not None:
            seen[q.name] = sig
    if gone: print(f"replaced: {len(gone)} waiting block file(s) not on the node's chain, from {min(gone)}", flush=True)
    return gone

def fetch_worker(k, lo, hi, jobs, d, window):
    for height in range(lo + k, hi + 1, jobs):
        p = d / f"block_{height}.json"
        if p.exists(): continue
        # files waiting below this height: only those count, so the block prep needs next is never held back
        while window and sum(1 for q in d.glob("block_*.json") if q.name[6:-5].isdigit() and int(q.name[6:-5]) < height) >= window:
            time.sleep(1)
        out, n_meta = build(height)
        write(out, p)
        print(f"block {height}: {len(out['txs'])} txs, {n_meta} prevouts", flush=True)

def supervise(lo, hi, d, window, jobs, od, follow):
    """range/follow with --after-saves: the fetch workers run as a batch over [start, top], and between polls this checks
    prep's chain (check_prep) and, when the node's best block changes, the waiting files (scan_waiting): the batch is
    stopped first, so no file fetched from the old chain is written after the scan. A batch starts after prep's newest
    COMPLETE save point, as the fetch always has: the files at or below it were consumed (state files above prep's real
    position, left by an older run, must not move where fetching starts)."""
    import multiprocessing
    refuse_if_marked(d)
    tip = PrepTip(od)
    batch, last_best, done_to, batch_top, seen = [], None, lo - 1, None, {}
    def stop_batch():
        for x in batch: x.terminate()
        for x in batch: x.join()
        batch.clear()
    try:
        while True:
            node_h, best = chain_info()
            if best != last_best:
                stop_batch()
                P = check_prep(d, od, tip)
                # above prep's tip only: the files from its last save point up to it are on the chain just checked,
                # and a restarted prep needs them
                gone = scan_waiting(d, tip.h, node_h)
                seen.clear()
                last_best = best
            else:
                P = check_prep(d, od, tip)
                # the files written since the last poll, against the chain as it is now (review L4)
                gone = scan_waiting(d, tip.h, node_h, seen)
            if gone: done_to = min(done_to, min(gone) - 1)
            if P is not None:
                # range: done once prep has SAVED hi (its last block is always a save point)
                if not follow and all((od / f"state_{hi}.{x}").exists() for x in ("json", "smt", "coinlog", "trie")):
                    print(f"prep has saved {hi}: done", flush=True)
                    return
                top = node_h if hi is None else min(hi, node_h)
                if batch and all(not x.is_alive() for x in batch):
                    bad = [x.exitcode for x in batch if x.exitcode != 0]
                    if bad: raise SystemExit(f"{len(bad)} fetch worker(s) failed: exit codes {bad}")
                    done_to = max(done_to, batch_top)
                    batch.clear()
                if not batch and done_to < top:
                    start = max(lo, max(complete_saves(od), default=0) + 1)
                    for q in d.glob("block_*.json.tmp"): q.unlink(missing_ok=True)   # a stopped worker's unfinished write
                    batch_top = top
                    batch[:] = [multiprocessing.Process(target=fetch_worker, args=(k, start, top, jobs, d, window)) for k in range(jobs)]
                    for x in batch: x.start()
            time.sleep(POLL_S)
    finally:
        stop_batch()

def write(out, path):
    tmp = f"{path}.tmp"
    with open(tmp, "w") as f: json.dump(out, f)
    os.replace(tmp, path)   # a consumer never reads half a file

def main():
    # range <lo> <hi> <dir> [--window N]: every block lo..hi into <dir>/block_<h>.json (existing files kept), waiting
    # while N block files are already waiting to be consumed (libzync-prep with HAZYNC_PREP_CONSUME=1 deletes each
    # after use), so a whole-chain prep never needs the whole chain's JSON on disk.
    if sys.argv[1] == "fork-point":
        # fork-point <prep out> [--deep]: where prep's chain and the node's part, without the J limit with --deep
        # (REORG.md §11 runbook step 2). Read-only.
        od = pathlib.Path(sys.argv[2])
        tip = PrepTip(od)
        P, hp = tip.get()
        node_h, best = chain_info()
        at = min(P, node_h)
        if P and at == P and rpc("getblockhash", [P]) == hp: fork, deepest = P, P
        else:
            J = tip.keep if tip.keep is not None else J_KEEP
            fork, deepest = walk_back(od, at, at if "--deep" in sys.argv else max(J - (P - at), 0))
        J = tip.keep if tip.keep is not None else J_KEEP
        depth = None if fork is None else P - fork
        reach = fork is not None and (tip.undo_from is None or fork + 1 >= tip.undo_from)
        print(json.dumps({"prep_tip": P, "prep_tip_from": tip.source, "node_tip": node_h, "fork": fork, "depth": depth,
                          "deepest_compared": deepest, "undo_from": tip.undo_from, "within_W": depth is not None and depth <= W_AUTO,
                          "within_J": depth is not None and depth <= J and reach}))
        return
    if sys.argv[1] == "follow":
        # follow <lo> <dir> --after-saves <prep out> [--hi H]: as range, to the node's tip as it moves (REORG.md §4)
        if "--after-saves" not in sys.argv: raise SystemExit("follow needs --after-saves <prep out>: it checks the chain prep is on")
        sys.argv[1:4] = ["range", sys.argv[2], sys.argv[sys.argv.index("--hi") + 1] if "--hi" in sys.argv else "follow", sys.argv[3]]
    if sys.argv[1] == "range":
        follow = sys.argv[3] == "follow"
        lo, hi, d = int(sys.argv[2]), None if follow else int(sys.argv[3]), pathlib.Path(sys.argv[4])
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
        if "--after-saves" in sys.argv:
            supervise(lo, hi, d, window, jobs, od, follow)
            return
        def worker(k): fetch_worker(k, lo, hi, jobs, d, window)
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
