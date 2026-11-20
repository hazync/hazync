#!/bin/bash
# The zAPP follower in TIP MODE (docs/libzync/ZAPP_FOLLOWER.md §5-§7, phases Z3-Z5): keeps zync-serve's tip mode
# (hazync-lz-zync-serve-tip) on the board's spine with NO stores of its own. It reads the prep run that holds the spine's
# height (the SOURCE): the board pass below the handover height X, the tip run (the paper prep) from X - 1 on (REORG.md
# §19, D1), and serves counting-trie facts from that run's disk trie at the root its prep logged for the spine's block.
# Each run:
#   1. copies the spine; `verify-any <spine> 1` says its height H; `verify-any <spine> 1 --check-end` against the source's
#      state_<H>.json (a view of that one file, as the coordinator does) binds it: the proof ends on that state, so its
#      trie_root is the root the proof commits (§5);
#   2. finds (H, the spine's block) in the source's trie/roots.log (the newest entry). Not there yet (prep has not
#      applied H, or no root of H was ever logged, e.g. below the source's first Z1 restart): keep serving, try again;
#   3. writes trie/PIN in the source (§4, §7): the lowest of the heights it will serve and, in the tip run, for a reorg
#      fall-back, the newest kept spine head at or below the tip run's position - W (else the oldest kept head). Prep then keeps every
#      slot those roots need (up to HAZYNC_TRIE_RETAIN_MAX). A pin written for a height prep already released holds
#      nothing for it (INTACT says so; zync-serve refuses it), but holds every height from the next flush on: the
#      first runs bootstrap the window the spine then grows into;
#   4. serves H and the newest earlier served height still on the source's chain (Z4: two heights, `?at=`): writes
#      at.json, restarts zync-serve, and if it does not stay up, tries H alone, then puts the previous set back and fails;
#   5. publishes proof.rcpt (the copied spine) once zync-serve serves H; drops the PIN of a source no served height and no
#      fall-back head needs any more (the board pass's, once the spine is past X).
# A REORG (Z5): the coordinator falls the spine back to a kept head H' (REORG.md §7); this run serves H' (repins) and
# drops every served height whose block the source no longer has. Prep's rewind bumped GEN and cut roots.log above the
# fork (P3), so zync-serve already answers 503 for the orphaned heights. H' no longer readable (past W): the run fails
# (alerts); facts resume when the board proves a head on the new branch.
# It proves nothing and is trusted for nothing: the page verifies the proof and checks every fact against it.
#
#   hazync-lz-zapp-follow-tip            (hazync-lz-zapp-follow-tip.timer runs it)
# Env: ZAPP_SERVE_DIR (at.json, proof.rcpt), COORD_SPINE, ZAPP_HOST (a libzync host: verify-any), ZAPP_WORK (scratch),
#   BOARD_OUT, BOARD_TRIE (the board pass), TIP_FROM (X; empty: no tip run), TIP_OUT, TIP_TRIE (the tip run),
#   ZAPP_PIN_W (288, REORG.md's W), ZAPP_KEEP (2 heights served).
# For tests: ZAPP_OWNER (the served files' owner, default hazync; empty: as is), ZAPP_RESTART / ZAPP_ACTIVE (restart
# zync-serve, ask whether it is up), ZAPP_SETTLE_S (5).
set -euo pipefail
SERVE=${ZAPP_SERVE_DIR:-/var/lib/hazync-lz/zapp-tip}
SPINE=${COORD_SPINE:-/var/lib/hazync-lz/spine}
HOST=${ZAPP_HOST:-/opt/hazync-lz/bin/host}
WORK=${ZAPP_WORK:-/srv/bulk/libzync-zapp-tip}
BOARD_OUT=${BOARD_OUT:-/var/lib/libzync-board/out}
BOARD_TRIE=${BOARD_TRIE:-/var/lib/libzync-board/trie}
TIP_FROM=${TIP_FROM:-}
TIP_OUT=${TIP_OUT:-/var/lib/libzync-fast/out}
TIP_TRIE=${TIP_TRIE:-/srv/bulk/libzync-prep/trie}
W=${ZAPP_PIN_W:-288}
KEEP=${ZAPP_KEEP:-2}
OWNER=${ZAPP_OWNER-hazync}
RESTART=${ZAPP_RESTART:-systemctl restart hazync-lz-zync-serve-tip}
ACTIVE=${ZAPP_ACTIVE:-systemctl is-active --quiet hazync-lz-zync-serve-tip}
log() { echo "[$(date -u +%FT%TZ)] $*"; }
die() { log "ERROR: $*"; exit 1; }
case "$TIP_FROM" in ''|*[!0-9]*) [ -z "$TIP_FROM" ] || die "TIP_FROM=$TIP_FROM: a height";; esac

mkdir -p "$WORK"
exec 9>"$WORK/.lock"; flock -n 9 || { log "another run holds the lock"; exit 0; }
# the source of height h: "<out> <trie>" (the run whose state_<h>.json the coordinator checks h against)
src_of() { if [ -n "$TIP_FROM" ] && [ "$1" -ge $((TIP_FROM - 1)) ]; then echo "$TIP_OUT $TIP_TRIE"; else echo "$BOARD_OUT $BOARD_TRIE"; fi; }

# 1. the proof and what it proves
if [ ! -s "$SPINE/spine.bin" ]; then
    # no spine: none yet, or a reorg below every kept head took it away (REORG.md §7) while facts are still served
    [ -s "$SERVE/proof.rcpt" ] && die "no spine, but facts are served at the proof in $SERVE: a reorg below every kept spine head? REORG.md §11"
    log "no spine yet"; exit 0
fi
cp "$SPINE/spine.bin" "$WORK/spine.bin"
H=$("$HOST" verify-any "$WORK/spine.bin" 1 2>/dev/null | grep -m1 '^RANGE-OK lo=1 .* anchored=1' | tr ' ' '\n' | sed -n 's/^hi=//p' || true)
[ -n "$H" ] || die "the spine does not verify as a genesis-anchored proof under this host's image"
read -r SOUT STRIE <<<"$(src_of "$H")"
rm -rf "$WORK/view"; mkdir -p "$WORK/view"
ln -s "$SOUT/state_$H.json" "$WORK/view/state_$H.json"
OKLINE=$(HAZYNC_LZ_STATE_DIR="$WORK/view" HAZYNC_BRIDGE_OUT="$WORK/view" "$HOST" verify-any "$WORK/spine.bin" 1 --check-end 2>/dev/null |
         grep -m1 '^RANGE-OK lo=1 .* end=board' || true)
BH=$(echo "$OKLINE" | tr ' ' '\n' | sed -n 's/^out_bhash=//p')
[ -n "$BH" ] || die "the spine (1..$H) does not end on the source's state_$H.json ($SOUT): not the chain the source holds"

# 2-5 in one place: reads roots.log, at.json, the spine heads and the sources' positions; writes the plan
PLAN=$(python3 - "$H" "$BH" "$SOUT" "$STRIE" "$SERVE/at.json" "$SPINE/heads" "$W" "$KEEP" "${TIP_FROM:-0}" \
                 "$BOARD_OUT" "$BOARD_TRIE" "$TIP_OUT" "$TIP_TRIE" <<'PY'
import json, os, struct, sys
H, BH, sout, strie, atf, heads_d = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5], sys.argv[6]
W, KEEP, X = int(sys.argv[7]), int(sys.argv[8]), int(sys.argv[9])
bo, bt, to, tt = sys.argv[10:14]
def src(h): return (to, tt) if X and h >= X - 1 else (bo, bt)
def state(d, h):
    try:
        j = json.load(open(os.path.join(d, f"state_{h}.json")))
        return bytes(j["tip_hash"]).hex(), bytes(j["trie_root"]).hex()
    except (OSError, ValueError, KeyError, TypeError):
        return None, None
def logged(trie, h, bh):
    try: b = open(os.path.join(trie, "roots.log"), "rb").read()
    except OSError: return False
    want = bytes.fromhex(bh)
    return any(struct.unpack_from("<I", b, 80 * i)[0] == h and b[80 * i + 4:80 * i + 36] == want for i in range(len(b) // 80))
def intact(trie):
    try: return int(open(os.path.join(trie, "INTACT")).read().split()[1])
    except (OSError, ValueError, IndexError): return 0
def pos(out):
    try: return int(json.load(open(os.path.join(out, "PREP_TIP.json")))["height"])
    except (OSError, ValueError, KeyError, TypeError): return None
tip_hash, root = state(sout, H)
if tip_hash != BH:
    print(json.dumps({"error": f"the source's state_{H}.json is block {tip_hash}, the spine's is {BH}"})); sys.exit()
try: served = json.load(open(atf))
except (OSError, ValueError): served = []
new = {"height": H, "hash": BH, "trie_root": root, "trie_dir": strie}
# earlier served heights still on their source's chain (a fall-back drops the orphaned ones), newest first
keep = [a for a in sorted(served, key=lambda a: -a["height"]) if a["height"] < H and state(src(a["height"])[0], a["height"])[0] == a["hash"]
        and logged(src(a["height"])[1], a["height"], a["hash"])]
plan = [new] + keep[:KEEP - 1]
# the pin per source trie: the lowest served height there, and the fall-back head (newest kept head <= P - W, else the
# oldest kept head) when it falls in that source
heads = sorted(int(n[6:-5]) for n in os.listdir(heads_d) if n.startswith("spine_") and n.endswith(".json") and n[6:-5].isdigit()) if os.path.isdir(heads_d) else []
pins = {}
for a in plan:
    pins[a["trie_dir"]] = min(pins.get(a["trie_dir"], a["height"]), a["height"])
# only for the tip run: a reorg reaches only the last W blocks below the node's tip, and the board pass is far below it
P = pos(sout)
if heads and P is not None and X and H >= X - 1:
    below = [h for h in heads if h <= P - W]
    fb = below[-1] if below else heads[0]
    if fb < H:
        d = src(fb)[1]
        pins[d] = min(pins.get(d, fb), fb)
servable = logged(strie, H, BH) and H >= intact(strie)
print(json.dumps({"plan": plan, "pins": pins, "servable": servable, "logged": logged(strie, H, BH), "intact_from": intact(strie),
                  "fell_back": any(a["height"] > H for a in served),
                  "same": [a["height"] for a in served] == [a["height"] for a in plan] and served == plan,
                  "all_dirs": sorted({bt, tt} if X else {bt})}))
PY
)
err=$(printf '%s' "$PLAN" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("error") or "")')
[ -z "$err" ] || die "$err"
field() { printf '%s' "$PLAN" | python3 -c "import json,sys; v=json.load(sys.stdin)$1; print(v if not isinstance(v,(list,dict,bool)) else json.dumps(v))"; }   # booleans: true / false

# 3. the pins first: a height is held from the next flush on, whether or not it can be served yet
own() { if [ -n "$OWNER" ]; then echo "-o $OWNER -g $OWNER"; fi; }
printf '%s' "$PLAN" | python3 -c '
import json, os, sys, time
p = json.load(sys.stdin)
for d in p["all_dirs"]:
    want = p["pins"].get(d)
    f = os.path.join(d, "PIN")
    if want is None:
        if os.path.exists(f):
            os.remove(f); print(f"unpinned {d}: no served height or fall-back head needs it")
        continue
    try: cur = int(open(f).read().split()[0])
    except (OSError, ValueError, IndexError): cur = None
    if cur != want:
        tmp = f + ".tmp"
        with open(tmp, "w") as o:
            o.write(f"{want} {int(time.time())}\n"); o.flush(); os.fsync(o.fileno())
        os.replace(tmp, f)
        print(f"PIN {d}: {cur} -> {want}")'

if [ "$(field '["servable"]')" != true ]; then
    # the spine fell back (a reorg, REORG.md §7) to a head this source cannot read any more: the served heights are the
    # orphaned branch's (zync-serve answers 503 for them); a person looks, facts resume at the next head (§11)
    [ "$(field '["fell_back"]')" != true ] ||
        die "the spine fell back to $H, whose root $STRIE does not hold (logged $(field '["logged"]'), intact from $(field '["intact_from"]')): a reorg deeper than the window; facts resume at the board's next head on the new branch (REORG.md §11)"
    if [ "$(field '["logged"]')" != true ]; then
        log "the spine is at $H (block ${BH:0:16}), which $STRIE/roots.log does not hold (yet): serving as before"; exit 0
    fi
    log "the root of $H is released in $STRIE (intact from $(field '["intact_from"]')); pinned now, served once the spine reaches a held height"; exit 0
fi
if [ "$(field '["same"]')" = true ] && [ -s "$SERVE/proof.rcpt" ] && cmp -s "$WORK/spine.bin" "$SERVE/proof.rcpt"; then
    log "at $H, nothing new"; exit 0
fi

# 4. at.json (H and the kept earlier heights), zync-serve restarted on it
# shellcheck disable=SC2046  # own prints "-o U -g U" or nothing: split on purpose
install -d $(own) "$SERVE"
try_set() {  # try_set <json>: zync-serve on that set; 0 if it stays up
    printf '%s\n' "$1" > "$SERVE/.at.json.new"
    # shellcheck disable=SC2046  # as above
    install -m 0644 $(own) "$SERVE/.at.json.new" "$SERVE/.at.json.inst" && rm -f "$SERVE/.at.json.new"
    mv -f "$SERVE/.at.json.inst" "$SERVE/at.json"
    $RESTART
    sleep "${ZAPP_SETTLE_S:-5}"
    $ACTIVE
}
[ -e "$SERVE/at.json" ] && cp -p "$SERVE/at.json" "$SERVE/.at.json.prev"
if ! try_set "$(field '["plan"]')"; then
    log "zync-serve did not stay up on $(field '["plan"]' | python3 -c 'import json,sys; print([a["height"] for a in json.load(sys.stdin)])'): trying $H alone"
    if ! try_set "$(field '["plan"][:1]')"; then
        if [ -e "$SERVE/.at.json.prev" ]; then mv -f "$SERVE/.at.json.prev" "$SERVE/at.json"; $RESTART; fi
        die "zync-serve did not stay up on the root of $H in $STRIE: the previous set is restored"
    fi
fi
# 5. the proof the facts are at, once they are served
# shellcheck disable=SC2046  # as above
install -m 0644 $(own) "$WORK/spine.bin" "$SERVE/.proof.rcpt.new"
mv -f "$SERVE/.proof.rcpt.new" "$SERVE/proof.rcpt"
log "serving facts at $(python3 -c 'import json,sys; print([a["height"] for a in json.load(open(sys.argv[1]))])' "$SERVE/at.json") from $STRIE (proof: the spine of 1..$H)"
