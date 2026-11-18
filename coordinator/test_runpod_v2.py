#!/usr/bin/env python3
"""The RunPod client speaks REST API v2 (api.runpod.io/v2) and keeps every GraphQL-era behaviour.

⛔ WHY. RunPod deprecated its GraphQL API (retiring "early 2027"). Every caller moved to v2, whose shapes
differ in ways that fail QUIETLY rather than loudly:

  * a capacity miss is now HTTP 400 with only a human `detail`. Read as an error, the bot would count every
    out-of-stock card as a "refusal" and latch itself off after three; read as a capacity miss everything
    else (a Cloudflare 403, a 402 balance, a 422 contract breach) would hide behind "no GPU capacity" --
    which is the bug sponsor_bot's refusal logic was written for (Cloudflare 1010, 2026-09-14);
  * a port has no `isIpPublic`; a private (10.x) entry taken for SSH means a pod that never boots;
  * GET /v2/pods may carry no live port mapping at all, so a bot that only reads the listing never sees SSH
    and terminates every pod it rents at the SSH timeout;
  * availability `NONE` is a truthy string where GraphQL's out-of-stock was null, so an unsellable card
    would be ranked and rented;
  * a 429 is a per-user quota (180/min) that GraphQL never hit the same way.

The client runs against a fake v2 server over real HTTP, as test_sponsor_bot.py does for the whole bot.
The final section sweeps the repo: no code may still call GraphQL, except the one documented v2 gap
(account balance, `clientBalance`, behind `gqlbal`).

  python3 test_runpod_v2.py            # must PASS
  python3 test_runpod_v2.py --control  # the v2 mapping put back wrong; the named checks MUST fail
"""
import http.server
import json
import os
import pathlib
import re
import sys
import tempfile
import threading
import urllib.parse

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import sponsor_bot as sb   # noqa: E402

fails = []


def check(ok, what):
    print(f"  {'ok  ' if ok else 'FAIL'} {what}")
    if not ok:
        fails.append(what)


import ipaddress   # noqa: E402

OLD_PUBLIC_RULE = "(not ipaddress.ip_address(ip).is_private)"      # review H1: 100.65.x.x passes it

if CONTROL:
    # ⚠ THE CONTROLS MUTATE THE SHIPPED CLIENT, one v2 difference each.
    sb.NO_CAPACITY_STATUS = None                      # a 400 read as a refusal, as a naive port would
    sb._public_ip = lambda ip: not ipaddress.ip_address(str(ip)).is_private   # H1: "not private" is not "public"
    sb.RATE_LIMIT_RETRIES = 0                         # a 429 surfaces as a refusal
    sb.RunPod.ssh_of = lambda self, pod_id: None      # only the listing is read for SSH
    sb.REQUEST_REFUSALS = ()                          # M1: a 401/402/422 hidden behind another type's 400
    sb.CAPACITY_LATCH_S = 0                           # H1(r2): a drought latches on the first miss
    sb.Bot._lost_create = lambda self, c, e: None      # M4(r3): a lost create is shrugged off
    sb.RUNPOD_REFUSALS_MAX = 10 ** 6                  # M1(r3): an over-ceiling pod never adds up to a latch
    sb.Bot._settle = lambda self, c, p, ok, note: setattr(p, "terminated", 1.0)   # M3(r2): unconfirmed = gone
    sb.Bot._sweep_leftovers = lambda self, c: None    # M2(r2): an earlier run's pods are never swept

    def _lenient_pods(self):                          # L3(r2): a 200 without a listing reads as "no pods" (r1)
        out, cursor = [], None
        while True:
            d = self._req("GET", "/pods", query=dict({"limit": 1000}, **({"cursor": cursor} if cursor else {}))) or {}
            out += [self._row(x) for x in (d.get("pods") or []) if x.get("id")]
            pg = d.get("pagination") or {}
            cursor = pg.get("nextCursor")
            if not pg.get("hasNextPage") or not cursor:
                return out
    sb.RunPod.pods = _lenient_pods

    def _serial(api, ids, sleep=None, tries=6, wait_s=5.0, budget_s=None, clock=None):   # M1(r2): one by one
        done, left = [], []
        for i in ids:
            try:
                api.terminate(i)
                (done if i not in {x["id"] for x in api.pods()} else left).append(i)
            except Exception:                         # noqa: BLE001
                left.append(i)
        return done, left
    sb.terminate_many = _serial
    sb.RATE_LIMITED = -1                              # M2: a 429 counted as a refusal / ends the terminate tries
    sb.Bot._price_ok = lambda self, c, p, info: True  # L2: a price unknown at deploy is never checked again


PUBLIC_IP = "45.23.12.1"         # RunPod's documentation example; nothing here dials it


class FakeV2:
    """Just enough of api.runpod.io/v2 to drive the client: scripted create answers, pods, catalogue."""

    def __init__(self):
        self.reqs = []                                   # (method, path, query, body, headers)
        self.create_answers = []                         # per create: (status, body) -- default 201 pod
        self.pods = {}
        self.list_runtime = False
        self.page_size = 1000
        self.rate_limit = 0                              # answer this many requests 429 first
        self.delete_status = {}                          # pod id -> forced status for DELETE
        self.create_then_fail = 0                        # creates that make the pod but answer 500 (lost answer)
        self.bad_listing = False                         # GET /v2/pods answers 200 without a listing
        self.create_cost = 0.74                          # what a new pod's `cost` reads at first
        self.cost_after = 0.74                           # ... and on every later GET
        self.catalog = [
            {"id": "NVIDIA GeForce RTX 4090", "name": "RTX 4090", "memory": 24, "secure": True, "community": True,
             "manufacturer": "NVIDIA", "price": {"secure": 0.89, "community": 0.34}, "availability": "LOW"},
            {"id": "NVIDIA A100 80GB PCIe", "name": "A100 PCIe", "memory": 80, "secure": True, "community": True,
             "manufacturer": "NVIDIA", "price": {"secure": 1.64, "community": 1.19}, "availability": "NONE"},
            {"id": "NVIDIA L40S", "name": "L40S", "memory": 48, "secure": True, "community": False,
             "manufacturer": "NVIDIA", "price": {"secure": 1.09, "community": 0}, "availability": "MEDIUM"},
        ]
        fake = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _go(self, method):
                n = int(self.headers.get("Content-Length") or 0)
                body = json.loads(self.rfile.read(n)) if n else None
                u = urllib.parse.urlsplit(self.path)
                q = dict(urllib.parse.parse_qsl(u.query))
                fake.reqs.append((method, u.path, q, body, dict(self.headers)))
                code, obj, hdrs = fake.answer(method, u.path, q, body)
                out = b"" if obj is None else (obj if isinstance(obj, bytes) else json.dumps(obj).encode())
                self.send_response(code)
                for k, v in hdrs:
                    self.send_header(k, v)
                if out:
                    self.send_header("Content-Type", "text/plain" if isinstance(obj, bytes) else
                                     "application/problem+json" if code >= 400 else "application/json")
                self.send_header("Content-Length", str(len(out)))
                self.end_headers()
                if out:
                    self.wfile.write(out)

            def do_GET(self):
                self._go("GET")

            def do_POST(self):
                self._go("POST")

            def do_DELETE(self):
                self._go("DELETE")

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}/v2"

    @staticmethod
    def problem(code, detail):
        return code, {"title": "x", "status": code, "detail": detail}, ()

    def pod_obj(self, pid, runtime=True):
        p = self.pods[pid]
        return {"id": pid, "name": p["name"], "status": "RUNNING", "cost": p["cost"], "cloud": "SECURE",
                "gpu": {"id": p["gpu"], "count": 1}, "dataCenterId": p.get("dc"),
                "ssh": {"proxy": None, "direct": p.get("direct")},
                "runtime": ({"uptime": 120, "ports": p.get("ports", [])} if runtime else None)}

    def answer(self, method, path, q, body):
        if self.rate_limit > 0:
            self.rate_limit -= 1
            code, obj, _ = self.problem(429, "minute window exceeded")
            return code, obj, (("Retry-After", "2"),)
        if method == "POST" and path == "/v2/pods":
            if self.create_answers:
                code, obj = self.create_answers.pop(0)
                if code != 201:
                    return code, obj, ()
            pid = f"pod{len(self.pods) + 1}"
            self.pods[pid] = {"name": body["name"], "gpu": body["gpu"]["id"], "cost": self.create_cost, "dc": None,
                              "ports": [{"private": 22, "public": 40022, "type": "tcp", "ip": PUBLIC_IP}]}
            obj = self.pod_obj(pid)
            self.pods[pid]["cost"] = self.cost_after
            if self.create_then_fail > 0:
                self.create_then_fail -= 1
                return self.problem(500, "upstream timeout")
            return 201, obj, ()
        if method == "GET" and path == "/v2/pods" and self.bad_listing:
            return 200, {"message": "ok"}, ()
        if method == "GET" and path == "/v2/pods":
            ids = sorted(self.pods)
            start = int(q.get("cursor") or 0)
            page = ids[start:start + self.page_size]
            more = start + self.page_size < len(ids)
            return 200, {"pods": [self.pod_obj(i, self.list_runtime) for i in page],
                         "pagination": {"hasNextPage": more, "nextCursor": str(start + self.page_size) if more else None}}, ()
        m = re.fullmatch(r"/v2/pods/([^/]+)", path)
        if m:
            pid = urllib.parse.unquote(m.group(1))
            if method == "DELETE" and pid in self.delete_status:
                return self.problem(self.delete_status[pid], "forced")
            if pid not in self.pods:
                return self.problem(404, "pod not found")
            if method == "GET":
                return 200, self.pod_obj(pid), ()
            if method == "DELETE":
                del self.pods[pid]
                return 204, None, ()
        if method == "GET" and path == "/v2/catalog/gpus" and self.catalog is None:
            return self.problem(500, "catalogue down")
        if method == "GET" and path == "/v2/catalog/gpus":
            stock = q.get("include") == "AVAILABILITY"
            return 200, {"gpus": [dict(g) if stock else {k: v for k, v in g.items() if k != "availability"}
                                  for g in self.catalog]}, ()
        return self.problem(404, "no route")


def client(fake, sleeps=None):
    return sb.RunPod("k-test", url=fake.url, timeout=10, sleep=(sleeps.append if sleeps is not None else (lambda s: None)))


A40, R4090 = "NVIDIA A40", "NVIDIA GeForce RTX 4090"
PUB = "ssh-ed25519 AAAATEST test@hazync"

# ── 1. the create body is the v2 contract, and SECURE ───────────────────────────────────────────────
print("== create: POST /v2/pods ==")
f = FakeV2()
got = client(f).deploy("hz-sponsor-1-1", PUB, gpu_types=(R4090,))
method, path, q, body, hdrs = f.reqs[-1]
check((method, path) == ("POST", "/v2/pods"), f"a deploy is POST /v2/pods ({method} {path})")
check(body == {"name": "hz-sponsor-1-1", "cloud": "SECURE", "gpu": {"id": R4090, "count": 1}, "image": sb.IMAGE,
               "disk": 40, "ports": ["22/tcp"], "env": {"PUBLIC_KEY": PUB}},
      f"the body is v2's nested shape: cloud SECURE, gpu{{id,count}}, env as an object, ports as a list ({body})")
check(hdrs.get("Authorization") == "Bearer k-test" and hdrs.get("User-Agent") == sb.USER_AGENT,
      "it carries the key as a Bearer token and the bot's own User-Agent (Cloudflare 1010)")
check(got == {"id": "pod1", "name": "hz-sponsor-1-1", "gpu_type": R4090, "cost_per_hr": 0.74},
      f"the result keeps its GraphQL-era shape; cost_per_hr is v2's `cost` ({got})")

# ── 2. ⛔ capacity miss vs refusal ───────────────────────────────────────────────────────────────────
print("== a 400 is a capacity miss, everything else a refusal ==")
f = FakeV2()
f.create_answers = [(400, {"title": "Bad Request", "status": 400, "detail": "There are no instances currently available"})]
got = client(f).deploy("hz-sponsor-1-2", PUB, gpu_types=(R4090, A40))
check(got is not None and got["gpu_type"] == A40,
      f"a 400 on the first type moves on to the next, like GraphQL's null answer did ({got})")
f = FakeV2()
f.create_answers = [(400, {"status": 400, "detail": "no capacity"})] * 2
f.catalog[0]["availability"] = "NONE"             # the catalogue agrees: no SECURE stock for either type
try:
    got, raised = client(f).deploy("hz-sponsor-1-3", PUB, gpu_types=(R4090, A40)), None
except sb.RunPodError as e:
    got, raised = None, e
check(raised is None and got is None,
      f"400 for every type is 'no capacity' (None), NOT a refusal that counts toward the latch ({raised})")
for code, what in ((403, "Cloudflare / pool access"), (402, "insufficient balance"), (401, "a bad key"),
                   (422, "a body the contract refuses"), (500, "a server fault")):
    f = FakeV2()
    f.create_answers = [(code, {"status": code, "detail": what})] * 2
    try:
        client(f).deploy("hz-sponsor-1-4", PUB, gpu_types=(R4090, A40))
        raised = None
    except sb.RunPodError as e:
        raised = e
    check(raised is not None and raised.status == code and what in str(raised),
          f"HTTP {code} ({what}) on every type is a REFUSAL carrying RunPod's detail ({raised})")
f = FakeV2()
f.create_answers = [(403, {"status": 403, "detail": "no access to this pool"}), (400, {"status": 400, "detail": "none"})]
try:
    got, raised = client(f).deploy("hz-sponsor-1-5", PUB, gpu_types=(R4090, A40)), None
except sb.RunPodError as e:
    got, raised = None, e
check(raised is None and got is None,
      "RunPod's own 403 ('no access to this pool') skips that type; a 400 on the next is an ANSWER (None)")
f = FakeV2()
f.create_answers = [(403, b"error code: 1010\n"), (201, None)]
try:
    got, raised = client(f).deploy("hz-sponsor-1-5b", PUB, gpu_types=(R4090, A40)), None
except sb.RunPodError as e:
    got, raised = None, e
check(raised is not None and raised.status == 403 and sum(1 for r in f.reqs if r[0] == "POST") == 1,
      f"a Cloudflare 403 (not RunPod's answer) is raised at once -- it refuses every type ({raised})")

# ── 2b. ⛔ review M1: request refusals are never masked, and an all-400 with stock is a refusal ─────────
print("== M1: a 401/402/422 is never hidden by another type's 400 ==")
for code in (401, 402, 422):
    for order in ("first", "second"):
        f = FakeV2()
        ans = [(code, {"status": code, "detail": f"simulated {code}"}), (400, {"status": 400, "detail": "none"})]
        f.create_answers = ans if order == "first" else ans[::-1]
        try:
            got, raised = client(f).deploy("hz-sponsor-m1", PUB, gpu_types=(R4090, A40)), None
        except sb.RunPodError as e:
            got, raised = None, e
        check(raised is not None and raised.status == code,
              f"M1: HTTP {code} on the {order} type is a refusal even when another type answered 400 ({raised})")
f = FakeV2()
f.create_answers = [(400, {"status": 400, "detail": "There are no instances currently available"})] * 2
api = client(f)          # f.catalog shows the 4090 at "LOW" -- the standing state (docs/OPERATIONS.md)
try:
    got, raised = api.deploy("hz-sponsor-m1b", PUB, gpu_types=(R4090, A40)), None
except sb.RunPodError as e:
    got, raised = None, e
check(raised is None and got is None,
      f"H1(r2): 400 on every type is a CAPACITY MISS even though the catalogue shows LOW stock ({raised})")
check(api.last_misses == [(R4090, "There are no instances currently available"),
                          (A40, "There are no instances currently available")],
      f"H1(r2): every 400's detail is kept for the log and the streak ({api.last_misses})")
check(not any(r[1] == "/v2/catalog/gpus" for r in f.reqs), "H1(r2): and no catalogue read decides it")


# ── 3. ⛔ 429 is retried after Retry-After ───────────────────────────────────────────────────────────
print("== the per-user quota ==")
f = FakeV2()
f.rate_limit = 2
sleeps = []
try:
    got = client(f, sleeps).deploy("hz-sponsor-1-6", PUB, gpu_types=(R4090,))
except sb.RunPodError as e:
    got = e
check(isinstance(got, dict) and sleeps == [2.0, 2.0],
      f"two 429s are waited out for Retry-After and the create then lands (slept {sleeps}, got {got})")
f = FakeV2()
f.rate_limit = 10
try:
    client(f).pods()
    raised = None
except sb.RunPodError as e:
    raised = e
check(raised is not None and raised.status == 429, f"a quota that stays exhausted is an error, not a hang ({raised})")

# ── 4. ⛔ listing, and the SSH mapping ───────────────────────────────────────────────────────────────
print("== GET /v2/pods and the port mapping ==")
f = FakeV2()
for i in range(5):
    f.pods[f"p{i}"] = {"name": f"hz-x-{i}", "gpu": A40, "cost": 0.49 + i,
                       "ports": [{"private": 22, "public": None, "type": "tcp", "ip": "10.1.2.3"},
                                 {"private": 22, "public": 41000 + i, "type": "tcp", "ip": "10.1.2.3"},
                                 {"private": 22, "public": 42000 + i, "type": "tcp", "ip": PUBLIC_IP},
                                 {"private": 9110, "public": 43000 + i, "type": "tcp", "ip": PUBLIC_IP}]}
f.page_size = 2
api = client(f)
rows = api.pods()
check(sorted(r["id"] for r in rows) == [f"p{i}" for i in range(5)] and
      sum(1 for r in f.reqs if r[1] == "/v2/pods") == 3,
      f"every page is followed to the end ({len(rows)} pods over "
      f"{sum(1 for r in f.reqs if r[1] == '/v2/pods')} requests)")
check({r["id"]: r["cost_per_hr"] for r in rows}["p3"] == 3.49, "cost_per_hr is each pod's `cost`")
check(all(r["ssh"] is None for r in rows),
      "a listing with no live runtime gives ssh None -- it does not invent a mapping")
check(api.ssh_of("p3") == (PUBLIC_IP, 42003),
      f"ssh_of asks GET /v2/pods/{{id}} and takes the PUBLIC 22 mapping, not a 10.x one ({api.ssh_of('p3')})")
check(api.ports_of("p3") == {22: (PUBLIC_IP, 42003), 9110: (PUBLIC_IP, 43003)},
      f"ports_of keeps exactly the public mappings, 9110 included ({api.ports_of('p3')})")
check(api.ports_of("gone") == {} and api.ssh_of("gone") is None, "a pod RunPod no longer has has no ports")
f.list_runtime = True
check({r["id"]: r["ssh"] for r in client(f).pods()}["p1"] == (PUBLIC_IP, 42001),
      "when the listing does carry runtime, its public mapping is used directly")
f.pods["p9"] = {"name": "hz-x-9", "gpu": A40, "cost": 1, "ports": [],
                "direct": {"host": PUBLIC_IP, "port": 47000, "username": "root", "command": "ssh"}}
check(client(f).ssh_of("p9") == (PUBLIC_IP, 47000), "with no runtime ports, RunPod's own ssh.direct is used")

# ── 5. the bot's SSH wait reads the pod when the listing has no mapping ──────────────────────────────
print("== the bot sees SSH that only GET /v2/pods/{id} reports ==")
f = FakeV2()
api = client(f)
info = api.deploy("hz-sponsor-7-1", PUB, gpu_types=(A40,))
with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), api, None, max_pods=1, max_usd=1, max_usd_per_hour=1, log=lambda m: None)
    pod = sb.Pod(info, 0.0)
    pod.state = "waiting_ssh"
    bot._live = lambda: [pod]
    bot._boot = lambda p: None
    bot._terminate = lambda c, p, why: setattr(p, "state", "terminated:" + why)
    bot.ssh_timeout_s = 10 ** 9
    bot._observe(None, 1.0)
check(pod.ssh == (PUBLIC_IP, 40022) and pod.state == "booting",
      f"a pod whose listing has no runtime still gets its SSH and boots ({pod.ssh}, {pod.state})")

# ── 6. terminate ─────────────────────────────────────────────────────────────────────────────────────
print("== DELETE /v2/pods/{id} ==")
f = FakeV2()
api = client(f)
info = api.deploy("hz-sponsor-8-1", PUB, gpu_types=(A40,))
check(sb.terminate_confirmed(api, info["id"], sleep=lambda s: None) and not f.pods,
      "terminate_confirmed deletes the pod and confirms it is no longer listed")
check(any(r[0] == "DELETE" and r[1] == f"/v2/pods/{info['id']}" for r in f.reqs), "termination is DELETE /v2/pods/{id}")
try:
    api.terminate("never-existed")
    raised = None
except sb.RunPodError as e:
    raised = e
check(raised is None, "terminating a pod RunPod no longer has (404) is not an error")
f.pods["stuck"] = {"name": "hz-sponsor-8-2", "gpu": A40, "cost": 1}
f.delete_status["stuck"] = 500
try:
    api.terminate("stuck")
    raised = None
except sb.RunPodError as e:
    raised = e
check(raised is not None and raised.status == 500, "a DELETE that fails for any other reason raises")
check(not sb.terminate_confirmed(api, "stuck", sleep=lambda s: None, tries=2),
      "and terminate_confirmed reports it unconfirmed while RunPod still lists it")

# ── 7. ⛔ the catalogue: availability, not a missing price, means out of stock ────────────────────────
print("== GET /v2/catalog/gpus ==")
import tip_smoke   # noqa: E402
f = FakeV2()
api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10)
econ = tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False)
econ.close()
ranked = tip_smoke.rank_card_types(api, economics=econ.name)
os.unlink(econ.name)
q = [r[2] for r in f.reqs if r[1] == "/v2/catalog/gpus"][-1]
check(q == {"include": "AVAILABILITY", "product": "POD", "cloud": "SECURE", "count": "1"},
      f"the ranking asks for SECURE pod stock for ONE GPU, as lowestPrice(gpuCount:1, secureCloud:true) did ({q})")
check([c["id"] for c in ranked] == [R4090, "NVIDIA L40S"],
      f"a type whose availability is NONE is not offered, though v2 still lists its price ({[c['id'] for c in ranked]})")
check(ranked[0]["display"] == "RTX 4090" and ranked[0]["price"] == 0.89 and ranked[0]["stock"] == "LOW",
      f"display is v2 `name` (== GraphQL displayName, measured), price is price.secure ({ranked[0]})")
offered = api.gpu_type_ids()
check(R4090 in offered and "NVIDIA A41" not in offered,
      "gpu_type_ids lists RunPod's ids, so a typo such as 'NVIDIA A41' is refused before renting")

# ── 8. tip_smoke's listening deploy ──────────────────────────────────────────────────────────────────
print("== tip_smoke.deploy_listening ==")
f = FakeV2()
api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10)
f.create_answers = [(400, {"status": 400, "detail": "none"})]
p = api.deploy_listening("hz-smoke-1", PUB, gpu_types=(R4090, A40))
body = [r[3] for r in f.reqs if r[0] == "POST"][-1]
check(body["ports"] == ["22/tcp", "9110/tcp"] and body["cloud"] == "SECURE" and body["disk"] == 40,
      f"the aggregate's pod publishes 9110 as well as 22, on SECURE ({body['ports']}, {body['cloud']})")
check(p and p["gpu_type"] == A40 and p["price"] == 0.74 and set(p) == {"id", "name", "gpu_type", "price", "dc"},
      f"it walks past a capacity miss and keeps its GraphQL-era result shape ({p})")

# ── H1. ⛔ RunPod's internal 100.65.x.x is NOT public, whatever its order ─────────────────────────────
print("== H1: carrier-grade NAT (100.64.0.0/10) is not a public address ==")
CGNAT = "100.65.12.34"           # the shape ~/libzync-gpu/live/fleet.txt recorded as isIpPublic:false
f = FakeV2()
for pid, order in (("first", [CGNAT, PUBLIC_IP]), ("last", [PUBLIC_IP, CGNAT])):
    f.pods[pid] = {"name": pid, "gpu": A40, "cost": 1,
                   "ports": [{"private": 22, "public": 50000 + i, "type": "tcp", "ip": ip} for i, ip in enumerate(order)]
                   + [{"private": 9110, "public": 51000 + i, "type": "tcp", "ip": ip} for i, ip in enumerate(order)]}
f.pods["only"] = {"name": "only", "gpu": A40, "cost": 1,
                  "ports": [{"private": 22, "public": 52000, "type": "tcp", "ip": CGNAT}],
                  "direct": {"host": CGNAT, "port": 52000, "username": "root", "command": "ssh"}}
api = client(f)
p22 = lambda pid: api.ports_of(pid).get(22)      # noqa: E731  (ports_of, not ssh_of: one mutation per check)
check(p22("first") == (PUBLIC_IP, 50001) and p22("last") == (PUBLIC_IP, 50000),
      f"H1: the GLOBAL 22 mapping wins whether the 100.65 entry comes first or last "
      f"({p22('first')}, {p22('last')})")
check(api.ports_of("first")[9110][0] == PUBLIC_IP and api.ports_of("last")[9110][0] == PUBLIC_IP,
      "H1: and the same for 9110")
check(p22("only") is None, f"H1: a pod with only a 100.65 address (runtime or ssh.direct) has NO public SSH ({p22('only')})")
f.pods["two"] = {"name": "two", "gpu": A40, "cost": 1, "ports": [
    {"private": 22, "public": 53000, "type": "tcp", "ip": PUBLIC_IP},
    {"private": 22, "public": 53001, "type": "tcp", "ip": "45.23.12.2"}]}
check(p22("two") == (PUBLIC_IP, 53000), "H1: between two global entries the FIRST is kept (deterministic)")

# The shell pickers (experimental bench scripts) run the same rule in an inline python3 -c. Run each one.
print("== H1: the shell scripts' port pickers ==")
import subprocess   # noqa: E402
ROOT = pathlib.Path(HERE).parent
BENCH = ROOT / "experimental" / "libzync-bench"
PICK = re.compile(r"python3 -c '\n?(import ipaddress,json,sys\n.*?)'", re.S)
sample = {"id": "p", "runtime": {"ports": [
    {"private": 22, "public": 40001, "type": "tcp", "ip": CGNAT},
    {"private": 9110, "public": 40002, "type": "tcp", "ip": CGNAT},
    {"private": 22, "public": 40003, "type": "tcp", "ip": PUBLIC_IP},
    {"private": 9110, "public": 40004, "type": "tcp", "ip": PUBLIC_IP},
    {"private": 22, "public": 40005, "type": "tcp", "ip": "45.23.12.2"},
    {"private": 9110, "public": 40006, "type": "tcp", "ip": "45.23.12.2"}]}}
pickers = []
for sh in sorted(BENCH.rglob("*.sh")):
    for m in PICK.finditer(sh.read_text()):
        pickers.append((sh.relative_to(ROOT), m.group(1)))
check(len(pickers) >= 9, f"H1: found the scripts' port pickers ({len(pickers)})")
for rel, code in pickers:
    if CONTROL:
        code = code.replace("ipaddress.ip_address(ip).is_global", OLD_PUBLIC_RULE)
    out = subprocess.run([sys.executable, "-c", code, "NAME"], input=json.dumps(sample),
                         capture_output=True, text=True).stdout.split()
    ok = (out[:2] == [PUBLIC_IP, "40003"] or out[:5] == ["NAME", PUBLIC_IP, "40003", PUBLIC_IP, "40004"])
    check(ok, f"H1: {rel} picks the first GLOBAL mapping, not 100.65 or a later one ({out[:6]})")
    if out[:1] == ["NAME"]:
        check(out[5:6] == [CGNAT], f"H1: {rel} records the internal 100.65 address as the private one ({out[5:6]})")
    err = subprocess.run([sys.executable, "-c", code, "NAME"], input=json.dumps({"status": 429, "title": "x"}),
                         capture_output=True, text=True).stdout.split()
    check(err in ([], ["ERR", "429"]),
          f"H1/M2: {rel} reads a 429 body as no mapping or as ERR 429, never as ports ({err})")
check(any(err == ["ERR", "429"] for err in
          [subprocess.run([sys.executable, "-c", c, "N"], input='{"status": 429}', capture_output=True,
                          text=True).stdout.split() for r, c in pickers if "podup" in str(r)]),
      "M2: podup.sh tells a 429 apart from 'no ports' (ERR 429), so it waits instead of giving up")

# ── M2. the shared quota: a 429 is waited out, never a refusal, never 'no ports' ─────────────────────
print("== M2: the quota ==")


class QuotaAPI:
    def __init__(self, limited):
        self.limited, self.listed, self.terminated = limited, 0, 0
        self.gone = False

    def terminate(self, pid):
        if self.limited > 0:
            self.limited -= 1
            raise sb.RunPodError("HTTP 429", status=429)
        self.terminated += 1
        self.gone = True

    def pods(self):
        self.listed += 1
        return [] if self.gone else [{"id": "pX"}]


class Clock:
    def __init__(self):
        self.t = 0.0
        self.sleeps = []

    def sleep(self, x):
        self.sleeps.append(x)
        self.t += x

    def __call__(self):
        return self.t


clk = Clock()
qa = QuotaAPI(limited=20)
ok = sb.terminate_confirmed(qa, "pX", sleep=clk.sleep, tries=6, wait_s=5, budget_s=10 ** 6, clock=clk)
check(ok and qa.terminated == 1,
      f"M2: twenty 429'd DELETEs do not use up terminate_confirmed's 6 tries -- the pod is still released "
      f"({ok}, waited {clk.t:.0f}s)")
clk = Clock()
qa = QuotaAPI(limited=10 ** 6)
ok = sb.terminate_confirmed(qa, "pX", sleep=clk.sleep, tries=6, wait_s=5, clock=clk)
check(not ok and clk.t <= sb.TERMINATE_BUDGET_S,
      f"M1(r2): a quota that NEVER clears ends in 'unconfirmed' within the WALL-CLOCK budget "
      f"({clk.t:.0f}s of {sb.TERMINATE_BUDGET_S}s)")


class OrderAPI:
    """Lists every pod until it has been DELETEd once; the first DELETE of `slow` answers 429 Retry-After 90."""
    def __init__(self, ids, slow=None):
        self.ids, self.calls, self.deleted, self.slow = set(ids), [], set(), slow

    def terminate(self, pid):
        self.calls.append(("DELETE", pid))
        if pid == self.slow:
            self.slow = None
            raise sb.RunPodError("HTTP 429", status=429, retry_after=90)
        self.deleted.add(pid)

    def pods(self):
        self.calls.append(("LIST", None))
        return [{"id": i, "name": i, "status": "RUNNING"} for i in self.ids - self.deleted]


oa = OrderAPI(["a", "b", "c"])
clk = Clock()
done, left = sb.terminate_many(oa, ["a", "b", "c"], sleep=clk.sleep, clock=clk)
check(oa.calls[:4] == [("DELETE", "a"), ("DELETE", "b"), ("DELETE", "c"), ("LIST", None)] and not left,
      f"M1(r2): every DELETE goes out FIRST, then one listing confirms them all ({oa.calls[:4]})")
oa = OrderAPI(["a", "b"], slow="b")
clk = Clock()
done, left = sb.terminate_many(oa, ["a", "b"], sleep=clk.sleep, clock=clk)
check(not left and 90 in clk.sleeps,
      f"M1(r2): a 429's Retry-After (90 s) is honoured, inside the budget ({clk.sleeps})")
f = FakeV2()
f.rate_limit = 10
try:
    client(f).pods()
    ra = None
except sb.RunPodError as e:
    ra = e.retry_after
check(ra == 2.0, f"M1(r2): the client carries Retry-After on the RunPodError it raises ({ra})")


class RL:
    ssh_pubkey = PUB


def launch_once(exc):
    class A:
        last_misses = []

        def deploy(self, name, pub):
            raise exc
    with tempfile.TemporaryDirectory() as d:
        logs = []
        bot = sb.Bot(os.path.join(d, "bot.db"), A(), RL(), max_pods=2, max_usd=10, max_usd_per_hour=5,
                     capacity_backoff_s=0, log=logs.append)
        real = sb.pending_work
        sb.pending_work = lambda *a, **k: [1]
        try:
            bot._launch(None, 1.0)
        finally:
            sb.pending_work = real
        return bot, logs


bot, logs = launch_once(sb.RunPodError("HTTP 429 Too Many Requests: minute window exceeded", status=429))
check(bot._refusals == 0 and bot.stop_reason is None,
      f"M2: a 429 on deploy is NOT a refusal -- it never moves the latch ({bot._refusals}, {logs[-1:]})")
bot, logs = launch_once(sb.RunPodError("HTTP 402 Payment Required: insufficient balance", status=402))
check(bot._refusals == 1, "a 402 on deploy still IS a refusal")


class SlowPorts:
    def __init__(self):
        self.n = 0

    def ports_of(self, pid):
        self.n += 1
        if self.n <= 2:
            raise sb.RunPodError("HTTP 429", status=429)
        return {22: (PUBLIC_IP, 40022)}


class Ready:
    def run(self, card, cmd):
        return "READY"


if CONTROL:
    tip_smoke._ports_or_none = lambda api, pod_id: api.ports_of(pod_id)        # M2: an error ends the wait
real_sleep, slept = tip_smoke.time.sleep, []
tip_smoke.time.sleep = slept.append
try:
    ready, _ = tip_smoke.wait_for_ssh(SlowPorts(), [{"id": "a", "name": "c1"}], Ready(), timeout_s=60)
    raised = None
except Exception as e:      # noqa: BLE001
    ready, raised = {}, e
finally:
    tip_smoke.time.sleep = real_sleep
check(raised is None and "c1" in ready,
      f"M2: wait_for_ssh survives 429s from ports_of and still brings the card up ({raised!r}, {sorted(ready)})")
backoffs = [x for x in slept if x != tip_smoke.PORTS_MIN_GAP_S]
check(backoffs[:2] == [20, 40], f"M2: and it BACKS OFF while RunPod refuses (slept {backoffs[:3]})")
check(slept.count(tip_smoke.PORTS_MIN_GAP_S) >= 3,
      f"M3(r2): and every poll is PACED for the 120/min sustained quota ({slept.count(tip_smoke.PORTS_MIN_GAP_S)} gaps)")

# ── L2. a price of 0 is unknown, not free ───────────────────────────────────────────────────────────
print("== L2: cost 0 while PROVISIONING ==")
for later, want_state in ((0.9, "terminated"), (0.3, "booting")):
    f = FakeV2()
    f.create_cost, f.cost_after = 0, later
    api = client(f)
    info = api.deploy("hz-sponsor-l2", PUB, gpu_types=(A40,))
    with tempfile.TemporaryDirectory() as d:
        bot = sb.Bot(os.path.join(d, "bot.db"), api, None, max_pods=1, max_usd=10, max_usd_per_hour=1,
                     pod_price_ceiling=0.5, log=lambda m: None)
        pod = sb.Pod(info, 0.0)
        bot._live = lambda: [pod]
        bot._boot = lambda p: None
        bot._terminate = lambda c, p, why: setattr(p, "state", "terminated")
        bot.ssh_timeout_s = 10 ** 9
        bot._observe(None, 1.0)
    check(info["cost_per_hr"] == 0 and pod.state == want_state and pod.cost_per_hr == later,
          f"L2: a pod deployed at cost 0 whose price turns out ${later}/h against a $0.50 ceiling is "
          f"{want_state} before it boots ({pod.state}, ${pod.cost_per_hr})")
f = FakeV2()
f.create_cost, f.cost_after = 0, 0.61
p = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10).deploy_listening("hz-smoke-l2", PUB, gpu_types=(A40,))
check(p["price"] == 0.61, f"L2: tip_smoke reads the price again when the create answered 0 ({p['price']})")

# ── M3 / L1. the bench scripts: bounded calls, loud unknowns ─────────────────────────────────────────
print("== M3 / L1: the bench scripts ==")
defs = []
for sh in sorted(BENCH.rglob("*.sh")):
    for n, line in enumerate(sh.read_text().split("\n"), 1):
        if re.match(r"\s*(rp|gqlbal) ?\(\)", line):
            if CONTROL and "pre970" in str(sh):
                line = line.replace("--max-time 20 ", "")
            defs.append((f"{sh.relative_to(ROOT)}:{n}", "--max-time" in line))
check(len(defs) >= 15 and all(ok for _, ok in defs),
      f"M3: every rp/gqlbal curl has --max-time ({[w for w, ok in defs if not ok][:4]}, {len(defs)} defs)")
bal = [str(sh.relative_to(ROOT)) for sh in BENCH.rglob("*.sh") if "|| echo 99" in sh.read_text()]
check(not bal, f"M3: no script turns an unreadable balance into a silent $99 ({bal})")
unread = [str(sh.relative_to(ROOT)) for sh in BENCH.rglob("*.sh")
          if "clientBalance" in sh.read_text() and "UNREADABLE" not in sh.read_text() and sh.name != "pod.sh"]
check(not unread, f"M3: every balance guard says UNREADABLE when it cannot read the balance ({unread})")

with tempfile.TemporaryDirectory() as home:
    os.makedirs(os.path.join(home, ".hazync"))
    open(os.path.join(home, ".hazync", "runpod.key"), "w").write("k-test")
    script = (BENCH / "gpu" / "fleet" / "pod.sh").read_text()
    if CONTROL:
        script = re.sub(r'\|\| \{ echo "pod.sh status: RunPod did not list pods.*?exit 1; \}', "|| true", script, flags=re.S)
        script = script.replace('lst["pods"]', '(lst.get("pods") or [])')      # the first version's reading
    path = os.path.join(home, "pod.sh")
    open(path, "w").write(script)
    f = FakeV2()
    f.pods["q1"] = {"name": "hz-q1", "gpu": A40, "cost": 1,
                    "ports": [{"private": 22, "public": 40022, "type": "tcp", "ip": PUBLIC_IP}]}
    env = dict(os.environ, HOME=home, RUNPOD_API=f.url, RUNPOD_GQL=f.url + "/no-graphql")
    r = subprocess.run(["bash", path, "status"], env=env, capture_output=True, text=True, timeout=60)
    good = json.loads(r.stdout) if r.returncode == 0 and r.stdout.strip() else {}
    check([p["id"] for p in good.get("pods", [])] == ["q1"] and good["pods"][0].get("runtime")
          and good.get("balance") is None,
          f"L1: pod.sh status lists the pods with their runtime filled in, balance null when unreadable "
          f"(rc {r.returncode})")
    f.rate_limit = 1
    r = subprocess.run(["bash", path, "status"], env=env, capture_output=True, text=True, timeout=60)
    check(r.returncode != 0 and not r.stdout.strip(),
          f"L1: a 429 listing makes pod.sh status FAIL, not print 'no pods' (rc {r.returncode}, {r.stdout[:60]!r})")

# ── r3 H1. a drought never latches; one identical 400 for CAPACITY_LATCH_S latches (exit 5 pages) ──────
print("== H1(r3): the capacity streak ==")
D1 = "There are no instances currently available"
with tempfile.TemporaryDirectory() as home:
    t0 = 1_000_000.0
    seen = {h / 2: sb.capacity_note(home, D1, t0 + h * 1800) for h in range(0, 15)}    # every 30 min, 0..7 h
    first = min((h for h, v in seen.items() if v), default=None)
    check(first == 6.0,
          f"H1(r3): one identical 400, seen every 30 min, latches at {sb.CAPACITY_LATCH_S // 3600} h, not before "
          f"(first latch at {first} h)")
    check(sb.capacity_note(home, "a different detail", t0 + 7.1 * 3600) is False,
          "H1(r2): a different detail starts a new streak")
    sb.capacity_note(home, D1, t0)
    check(sb.capacity_note(home, D1, t0 + 5 * 3600) is False,
          "H1(r2): a gap of over 2 h without a miss ends the streak (the stock came back, or nothing waited)")


class MissAPI:
    """deploy() always misses, every type with the same detail."""
    def __init__(self, details):
        self.details, self.last_misses, self.deploys = details, [], 0

    def deploy(self, name, pub):
        self.deploys += 1
        self.last_misses = [(f"T{i}", d) for i, d in enumerate(self.details)]
        return None


def looks(api, n, home=None, clock=None):
    with tempfile.TemporaryDirectory() as d:
        logs = []
        bot = sb.Bot(os.path.join(d, "bot.db"), api, RL(), max_pods=2, max_usd=10, max_usd_per_hour=5,
                     capacity_backoff_s=0, log=logs.append, clock=clock or time.time)
        bot.state_home = home
        real = sb.pending_work
        sb.pending_work = lambda *a, **k: [1]
        try:
            for _ in range(n):
                bot._launch(None, bot.clock())
        finally:
            sb.pending_work = real
        return bot, logs


import time   # noqa: E402
bot, logs = looks(MissAPI([D1, D1]), 3)
check(bot.stop_reason is None and bot._refusals == 0,
      f"H1(r2): three looks of a normal drought (LOW stock, all-400) neither latch nor page "
      f"({bot.stop_reason}, {bot._refusals})")
check(any(D1 in m for m in logs), "H1(r2): and the log says RunPod's own words for the miss")
with tempfile.TemporaryDirectory() as home:
    now = time.time()
    json.dump({"detail": D1, "first": now - 5 * 3600, "last": now - 300},
              open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(MissAPI([D1, D1]), 1, home=home)
    check(bot.stop_reason is None, f"H1(r3): 5 h of the same 400 does not stop the bot yet ({bot.stop_reason})")
    json.dump({"detail": D1, "first": now - 6.1 * 3600, "last": now - 300},
              open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(MissAPI([D1, D1]), 1, home=home)
    check(bot.stop_reason == "runpod" and any("SAME 400" in m for m in logs),
          f"H1(r3): 6 h of the same 400 stops it as 'runpod' -- exit 5, which latches and pages ({bot.stop_reason})")
    json.dump({"detail": D1, "first": now - 13 * 3600, "last": now - 300},
              open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(MissAPI([D1, "another reason"]), 1, home=home)
    check(bot.stop_reason is None and not os.path.exists(os.path.join(home, sb.CAPACITY_STATE)),
          "H1(r2): misses whose details DIFFER between types end the streak (that is capacity, not a request)")

# ── r2 M3. an unconfirmed termination is not a termination ────────────────────────────────────────────
print("== M3(r2): unconfirmed pods stay live ==")


class StuckAPI:
    """Rents fine; DELETE is accepted and the pod never leaves the listing."""
    def __init__(self):
        self.n, self.last_misses = 0, []

    def deploy(self, name, pub):
        self.n += 1
        return {"id": f"s{self.n}", "name": name, "gpu_type": A40, "cost_per_hr": 0.4}

    def terminate(self, pid):
        pass

    def pods(self):
        return [{"id": f"s{i}", "name": f"x{i}", "status": "RUNNING"} for i in range(1, self.n + 1)]

    def pod(self, pid):
        return None


with tempfile.TemporaryDirectory() as d:
    clk = Clock()
    clk.t = 1000.0
    sa = StuckAPI()
    bot = sb.Bot(os.path.join(d, "bot.db"), sa, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=lambda m: None, clock=clk, sleep=clk.sleep, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1, 2]
    try:
        bot._launch(c, clk())
        p1 = bot.pods[0]
        bot._terminate(c, p1, "test", budget_s=10)
        s1 = bot.spend(clk() + 3600)
        bot._launch(c, clk())
    finally:
        sb.pending_work = real
    row = c.execute("SELECT terminated_at FROM sponsor_pods WHERE pod_id='s1'").fetchone()
    c.close()
check(p1.live and p1.state == "unconfirmed" and "s1" in bot.unconfirmed,
      f"M3(r2): a pod RunPod still lists after terminate stays LIVE, state unconfirmed ({p1.live}, {p1.state})")
check(s1 >= 0.39, f"M3(r2): its spend keeps counting (${s1:.2f} an hour later)")
check(sa.n == 1, f"M3(r2): and no replacement is rented on top of it at --max-pods 1 ({sa.n} deploys)")
check(row is not None and row[0] is None, "M3(r2): its row keeps terminated_at NULL, for the next start to sweep")

with tempfile.TemporaryDirectory() as d:
    sa = StuckAPI()
    sa.n = 2                                             # s1, s2 left by an "earlier run"
    bot = sb.Bot(os.path.join(d, "bot.db"), sa, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    c.execute("INSERT INTO sponsor_pods(pod_id, name, gpu_type, cost_per_hr, created_at) VALUES('s1','hz-sponsor-1-1','A40',0.4,1)")
    c.commit()
    try:
        bot._sweep_leftovers(c)
        refused = None
    except sb.BotRefused as e:
        refused = e
    c.close()
check(refused is not None and "s1" in str(refused),
      f"M2(r2): the next start finds the earlier run's unconfirmed pod, cannot stop it, and REFUSES to rent ({refused})")

# ── r3 L1/M2/M4. a create whose answer was lost ───────────────────────────────────────────────────
print("== L1/M4(r3): a create whose answer was lost ==")
f = FakeV2()
f.create_then_fail = 1
try:
    client(f).deploy("hz-sponsor-l1", PUB, gpu_types=(R4090, A40))
    lost = None
except sb.LostCreate as e:
    lost = e
posts = [r for r in f.reqs if r[0] == "POST"]
check(lost is not None and lost.name == "hz-sponsor-l1" and len(posts) == 1,
      f"L1(r3): a 500 on create raises LostCreate naming the pod, and NO second type is rented ({lost}, {len(posts)} creates)")


class LostAPI:
    """Every deploy's answer is lost; the pod exists under the run's prefix (or the listing fails)."""
    def __init__(self, listing_ok=True):
        self.listing_ok, self.live, self.last_misses = listing_ok, {}, []

    def deploy(self, name, pub):
        self.live[f"L{len(self.live) + 1}"] = name
        raise sb.LostCreate(f"the create of {name} got no answer", status=None, name=name)

    def pods(self):
        if not self.listing_ok:
            raise sb.RunPodError("HTTP 503", status=503)
        return [{"id": i, "name": n, "status": "RUNNING"} for i, n in self.live.items()]

    def terminate(self, pid):
        self.live.pop(pid, None)


for ok_ in (True, False):
    la = LostAPI(listing_ok=ok_)
    with tempfile.TemporaryDirectory() as d:
        logs = []
        bot = sb.Bot(os.path.join(d, "bot.db"), la, RL(), max_pods=2, max_usd=10, max_usd_per_hour=5,
                     capacity_backoff_s=0, log=logs.append, sleep=lambda x: None, confirm_wait_s=0)
        real = sb.pending_work
        sb.pending_work = lambda *a, **k: [1]
        try:
            bot._launch(None, bot.clock())
        finally:
            sb.pending_work = real
    if ok_:
        check(not la.live and bot.stop_reason is None and bot._refusals == 0,
              f"M4(r3): the pod a lost create made under the run's own prefix is found and released at once "
              f"({la.live}, {bot.stop_reason})")
    else:
        check(bot.stop_reason == "runpod" and bot.maybe_billing and any("MAY BE BILLING" in m for m in logs),
              f"M4(r3): when it cannot be confirmed the run STOPS (exit 5) saying a pod may be billing "
              f"({bot.stop_reason}, {bot.maybe_billing})")

# ── r3 M1. a pod over the price ceiling is a refusal toward the latch ─────────────────────────────────
print("== M1(r3): over the price ceiling ==")


class DearAPI:
    def __init__(self):
        self.n, self.last_misses, self.gone = 0, [], set()

    def deploy(self, name, pub):
        self.n += 1
        return {"id": f"d{self.n}", "name": name, "gpu_type": A40, "cost_per_hr": 3.0}

    def terminate(self, pid):
        self.gone.add(pid)

    def pods(self):
        return [{"id": f"d{i}", "name": "x", "status": "RUNNING"} for i in range(1, self.n + 1)
                if f"d{i}" not in self.gone]

    def pod(self, pid):
        return None


da = DearAPI()
with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), da, RL(), max_pods=2, max_usd=100, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        for _ in range(5):
            if bot.stop_reason:
                break
            bot._launch(c, bot.clock())
    finally:
        sb.pending_work = real
    c.close()
check(bot.stop_reason == "runpod" and da.n == sb.RUNPOD_REFUSALS_MAX,
      f"M1(r3): a pod over the ceiling, again and again, latches after RUNPOD_REFUSALS_MAX ({da.n} rented, "
      f"{bot.stop_reason})")

print("== M5(r3): the run lock ==")
try:
    sb.acquire_run_lock("/nonexistent-hazync-dir/x")
    locked = "no error"
except SystemExit as e:
    locked = str(e)
check("refusing to run unlocked" in locked, f"M5(r3): a run lock that cannot be opened stops the run ({locked[:80]})")
with tempfile.TemporaryDirectory() as home:
    l1 = sb.acquire_run_lock(home)
    out = subprocess.run([sys.executable, "-c", f"import sys; sys.path.insert(0, {HERE!r}); import sponsor_bot as s; "
                          f"print(s.acquire_run_lock({home!r}))"], capture_output=True, text=True).stdout.strip()
    check(l1 not in (None, False) and out == "False", f"M5(r3): a second process cannot take a held lock ({out})")

# ── r2 L3. a 200 without a listing is not "no pods" ──────────────────────────────────────────────────
print("== L3(r2): a listing that is not a listing ==")
f = FakeV2()
f.pods["z1"] = {"name": "z1", "gpu": A40, "cost": 1}
f.bad_listing = True
try:
    client(f).pods()
    raised = None
except sb.RunPodError as e:
    raised = e
check(raised is not None, f"L3(r2): GET /v2/pods answering 200 without `pods`/`pagination` RAISES ({raised})")
f.delete_status["z1"] = 500
clk = Clock()
check(not sb.terminate_confirmed(client(f), "z1", sleep=clk.sleep, clock=clk),
      "L3(r2): so a termination is never 'confirmed' by a listing that was not one")

# ── r3 M2. tip_smoke: a lost create is found by name only among pods NEW since the run began ──────────
print("== M2(r3): tip_smoke's lost create ==")
if CONTROL:
    tip_smoke.SmokeRunPod._find_lost = lambda self, name, gpu_types: None
f = FakeV2()
f.pods["old1"] = {"name": "hz-smoke-1", "gpu": A40, "cost": 1}          # an earlier run's pod of the same name
api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10)
f.create_then_fail = 1
pp = api.deploy_listening("hz-smoke-1", PUB, gpu_types=(R4090, A40))
posts = [r for r in f.reqs if r[0] == "POST"]
check(pp is not None and pp["id"] != "old1" and len(posts) == 1,
      f"M2(r3): the lost create's NEW pod is found, never the earlier run's pod of the same name ({pp}, "
      f"{len(posts)} creates)")

# ── r2 C1. a rent that stops half way: every pod tracked, released, and the record kept until the account agrees
print("== C1(r2): rent_uniform and the teardown ==")


class RentAPI:
    def __init__(self, ok, then, listing_fails=False):
        self.ok, self.then, self.n, self.live, self.listing_fails = ok, then, 0, set(), listing_fails

    def deploy_listening(self, name, pub, gpu_types):
        if self.n >= self.ok:
            if self.then == "miss":
                return None
            raise sb.RunPodError("HTTP 402 Payment Required: insufficient balance", status=402)
        self.n += 1
        self.live.add(f"r{self.n}")
        return {"id": f"r{self.n}", "name": name, "gpu_type": gpu_types[0], "price": 0.5, "dc": None}

    def terminate(self, pid):
        self.live.discard(pid)

    def pods(self):
        if self.listing_fails:
            raise sb.RunPodError("HTTP 503", status=503)
        return [{"id": i, "name": i, "status": "RUNNING"} for i in self.live]


if CONTROL:
    def _old_rent(api, gt, upto, pub, rented_path, created, unreleased, prefix=tip_smoke.PREFIX):
        got = []
        for i in range(upto):
            p_ = api.deploy_listening(f"{prefix}{i+1}", pub, gpu_types=(gt,))
            if not p_:
                break
            got.append(p_)
            with open(rented_path, "w") as fh:
                json.dump(got, fh)
        return got
    tip_smoke.rent_uniform = _old_rent

    def _old_release(api, rented_path, created, unreleased):      # C1(r2): release `created`, check `created`
        for x in created:
            api.terminate(x["id"])
        left = [x["id"] for x in created if x["id"] in {y["id"] for y in api.pods()}]
        if not left:
            os.path.exists(rented_path) and os.unlink(rented_path)
        return left
    tip_smoke.release_recorded = _old_release
with tempfile.TemporaryDirectory() as d:
    rp_ = os.path.join(d, "rented.json")
    ra = RentAPI(ok=3, then="402")
    created, unreleased = [], []
    try:
        tip_smoke.rent_uniform(ra, A40, 5, PUB, rp_, created, unreleased)
        raised = None
    except sb.RunPodError as e:
        raised = e
    check(raised is not None and sorted(p["id"] for p in created) == ["r1", "r2", "r3"],
          f"C1(r2): 3 rented then a 402: all 3 are handed to the tracked set ({[p['id'] for p in created]})")
    left = tip_smoke.release_recorded(ra, rp_, [], [])
    check(left == [] and not ra.live and not os.path.exists(rp_),
          f"C1(r2): the teardown releases them FROM THE RECORD even when `created` is empty, then drops the record "
          f"({left}, live {sorted(ra.live)})")
with tempfile.TemporaryDirectory() as d:
    rp_ = os.path.join(d, "rented.json")
    ra = RentAPI(ok=3, then="miss")
    created, unreleased = [], []
    got = tip_smoke.rent_uniform(ra, A40, 5, PUB, rp_, created, unreleased)
    check(len(got) == 3, f"C1(r2): 3 rented then a 400 (a miss) returns the 3 -- no exception, nothing lost ({len(got)})")
    ra.listing_fails = True
    left = tip_smoke.release_recorded(ra, rp_, [], [])
    check(left is None and os.path.exists(rp_),
          "C1(r2): when the account cannot be listed the record is KEPT (never 'account check: clean')")

# ── r2 L1 / M4. the bench scripts' create and terminate helpers, against a stub `rp` ─────────────────
print("== L1/M4(r2): rp_create and rp_term in the bench scripts ==")
HELP = re.compile(r"(rp_create\(\)\{.*?\n  printf '%s\\n' \"\$ID\"; \}\n.*?rp_term\(\)\{.*?\n  return 1; \})", re.S)
helper_texts = {}
for sh in sorted(BENCH.rglob("*.sh")):
    t = sh.read_text()
    if "rp_create(){" in t or "rp_term(){" in t:
        m = HELP.search(t)
        helper_texts[str(sh.relative_to(ROOT))] = m.group(1) if m else None
check(len(helper_texts) >= 11 and len(set(helper_texts.values())) == 1 and None not in helper_texts.values(),
      f"M4(r2): every bench script carries the SAME rp_create/rp_term ({len(helper_texts)} scripts, "
      f"{len(set(helper_texts.values()))} versions)")
raw_calls = []
for sh in sorted(BENCH.rglob("*.sh")):
    for n, line in enumerate(sh.read_text().split("\n"), 1):
        if re.search(r"\brp (POST /pods|DELETE )", line) and "rp_create(){" not in line:
            raw_calls.append(f"{sh.relative_to(ROOT)}:{n}")
inside = {k for k in raw_calls}
check(len(raw_calls) == 2 * len(helper_texts),
      f"M4(r2): no script creates or deletes a pod except through rp_create / rp_term ({len(raw_calls)} raw calls, "
      f"expected {2 * len(helper_texts)}, the helpers' own)")
helpers = next(iter(helper_texts.values()))
if CONTROL:
    helpers = helpers.replace("ID=$(rp GET '/pods?limit=1000'", "ID=-; : $(rp GET '/pods?limit=1000'")   # L1: no lookup
    helpers = helpers.replace("for t in $(seq 1 20); do", "for t in 1; do")               # M4: single-shot
    helpers = helpers.replace('    for id in $left; do rp DELETE "/pods/$id" > /dev/null; done\n', "")   # H3: serial
    helpers = helpers.replace('    still=""\n    for id in $left; do\n',
                              '    still=""\n    for id in $left; do rp DELETE "/pods/$id" > /dev/null\n')
    helpers = helpers.replace('case "$R" in *"error code: 10"*)', 'case "$R" in *"NEVER"*)')   # L: Cloudflare not fatal
    helpers = helpers.replace('"") echo "⛔ create of $2: no answer', '"") return 1; echo "x')       # M3: unknown = miss


def bash_with(stub, body, timeout=30):
    script = "sleep(){ :; }\n" + stub + "\n" + helpers + "\n" + body
    r = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=timeout)
    return r.returncode, r.stdout.strip(), r.stderr


STUB = r"""rp(){ case "$1 $2" in
  "POST /pods") cat "$T/post" ;;
  "GET /pods?limit=1000") cat "$T/list" ;;
  "DELETE "*) echo "" ;;
  "GET /pods/"*) n=$(cat "$T/n" 2>/dev/null || echo 0); echo $((n+1)) > "$T/n"
                 if [ $n -ge $(cat "$T/gone_after") ]; then echo '{"title":"Not Found","status":404,"detail":"pod not found"}'
                 else echo '{"id":"x","status":"RUNNING"}'; fi ;;
esac; }"""
with tempfile.TemporaryDirectory() as T:
    os.environ["T"] = T

    def setup(post, listing='{"pods":[],"pagination":{"hasNextPage":false,"nextCursor":null}}', gone_after=0):
        open(os.path.join(T, "post"), "w").write(post)
        open(os.path.join(T, "list"), "w").write(listing)
        open(os.path.join(T, "gone_after"), "w").write(str(gone_after))
        if os.path.exists(os.path.join(T, "n")):
            os.unlink(os.path.join(T, "n"))

    setup('{"id":"pod123","name":"lz-x","status":"PROVISIONING","cost":0.5}')
    check(bash_with(STUB, 'rp_create "{}" lz-x')[:2] == (0, "pod123"), "L1(r2): rp_create prints a created pod's id")
    setup('{"title":"Internal Server Error","status":500,"detail":"upstream timeout"}',
          '{"pods":[{"id":"podLOST","name":"lz-x","status":"PROVISIONING"}],"pagination":{"hasNextPage":false}}')
    rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
    check((rc, out) == (0, "podLOST"), f"L1(r2): a 500 on create looks the pod up BY NAME and returns it ({rc}, {out!r})")
    setup("")
    rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
    check(rc == 1 and out == "", f"L1(r2): no answer and no such pod is 'no pod' (rc 1) ({rc}, {out!r})")
    for code, want in ((400, 1), (402, 2), (401, 2), (422, 2), (429, 3)):
        setup('{"title":"x","status":%d,"detail":"d"}' % code)
        rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
        check(rc == want and out == "", f"L4(r2): HTTP {code} on create exits {want} ({rc})")
    setup("error code: 1010")
    rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
    check(rc == 2 and out == "", f"L(r3): a Cloudflare page is FATAL (2), not 'no capacity' ({rc})")
    setup('{"title":"x","status":502,"detail":"bad gateway"}', '{"message": "not a listing"}')
    rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
    check(rc == 4 and "MAY BE BILLING" in err,
          f"M3(r3): a lost answer whose follow-up listing FAILS exits 4 (may be billing), not 1 ({rc})")
    setup('{"title":"x","status":500,"detail":"y"}',
          '{"pods":[{"id":"pA","name":"lz-x","status":"RUNNING"},{"id":"pB","name":"lz-x","status":"RUNNING"},'
          '{"id":"pC","name":"lz-x","status":"TERMINATED"}],"pagination":{"hasNextPage":false}}')
    rc, out, err = bash_with(STUB, 'rp_create "{}" lz-x')
    check(rc == 0 and out == "pA" and "releasing the rest" in err,
          f"M2(r3): two live pods under one lost name: the first is kept, the other RELEASED, a TERMINATED one ignored "
          f"({rc}, {out!r})")
    setup("", gone_after=2)
    rc, out, err = bash_with(STUB, 'rp_term pod9')
    check(rc == 0 and "confirmed gone" in err, f"M4(r2): rp_term retries until RunPod answers 404 ({rc})")
    setup("", gone_after=10 ** 6)
    rc, out, err = bash_with(STUB, 'rp_term pod9')
    check(rc == 1 and "NOT CONFIRMED" in err, f"M4(r2): and is LOUD (exit 1) when the pod never goes ({rc})")
    rc, out, err = bash_with(STUB, 'rp_term ""')
    check(rc == 2 and "no pod id" in err, f"L(r3): rp_term with an empty id refuses (2) instead of DELETE /pods/ ({rc})")
    ORDER = STUB.replace('rp(){ case "$1 $2" in', 'rp(){ echo "$1 $2" >> "$T/order"; case "$1 $2" in')
    setup("", gone_after=0)
    if os.path.exists(os.path.join(T, "order")):
        os.unlink(os.path.join(T, "order"))
    rc, out, err = bash_with(ORDER, 'rp_term a b c')
    order = open(os.path.join(T, "order")).read().split("\n")[:4]
    check(rc == 0 and order[:3] == ["DELETE /pods/a", "DELETE /pods/b", "DELETE /pods/c"],
          f"H3(r3): rp_term a b c sends EVERY DELETE first, then confirms ({order})")
    del os.environ["T"]

# ── 9. ⛔ nothing in the repo still calls GraphQL ─────────────────────────────────────────────────────
print("== the repo no longer calls RunPod GraphQL ==")
ROOT = pathlib.Path(HERE).parent
SKIP = {".git", "node_modules", "target", "__pycache__", "venv", ".venv"}
SELF = os.path.basename(__file__)
# ⚠ The fake server and the cloud-type scanner NAME the GraphQL operations in prose and patterns; they call
# nothing. Everything else is in scope: the bug class is a caller nobody remembered.
NOT_CALLERS = {SELF, "test_pod_cloud_type.py"}
OPS = re.compile(r"api\.runpod\.io/graphql|podFindAndDeployOnDemand\s*\(|podTerminate\s*\(|myself\s*\{\s*pods|"
                 r"gpuTypes\s*(\(|\{)|lowestPrice\s*\(")
left = []
for p in ROOT.rglob("*"):
    if p.is_dir() or p.suffix not in (".py", ".sh", ".js", ".ts") or p.name in NOT_CALLERS:
        continue
    if any(part in SKIP for part in p.parts):
        continue
    for n, line in enumerate(p.read_text(errors="ignore").split("\n"), 1):
        # The ONE allowed GraphQL call: the account balance, which v2 does not offer (`gqlbal`).
        # Comments that NAME the old operations (the client's GraphQL->v2 mapping table) call nothing.
        if line.lstrip().startswith("#"):
            continue
        if OPS.search(line) and "gqlbal" not in line:
            left.append(f"{p.relative_to(ROOT)}:{n}")
check(not left, f"no code calls RunPod GraphQL outside the balance gap ({left[:6]}{' ...' if len(left) > 6 else ''})")

print()
# ⚠ NAME EVERY ONE AND COUNT THEM: each is the consequence of one mutation above (some share a cause).
EXPECTED_CONTROL = {
    "400 for every type is 'no capacity'",
    "C1(r2): 3 rented then a 402: all 3 are handed to the tracked set",
    "C1(r2): the teardown releases them FROM THE RECORD even when `created` is empty, then drops the record",
    "C1(r2): when the account cannot be listed the record is KEPT",
    "H1(r2): 400 on every type is a CAPACITY MISS even though the catalogue shows LOW stock",
    "H1(r2): a different detail starts a new streak",
    "H1(r2): a gap of over 2 h without a miss ends the streak",
    "H1(r2): every 400's detail is kept for the log and the streak",
    "H1(r2): three looks of a normal drought",
    "H1(r3): 5 h of the same 400 does not stop the bot yet",
    "H1(r3): one identical 400, seen every 30 min, latches at 0 h, not before",
    "H1: a pod with only a 100.65 address",
    "H1: and the same for 9110",
    "H1: experimental/libzync-bench/gpu/fleet/live/grab2.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/gpu/fleet/live/grab2.sh records the internal 100.65 address as the private one",
    "H1: experimental/libzync-bench/gpu/fleet/live/podup.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/gpu/fleet/live/podup.sh records the internal 100.65 address as the private one",
    "H1: experimental/libzync-bench/gpu/fleet/podup.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/gpu/fleet/podup.sh records the internal 100.65 address as the private one",
    "H1: experimental/libzync-bench/gpu/run_c14_pod.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/gpu/run_paper_pod.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/headers/run_headers_pod.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/pre970/gpu_run.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/signet/genesis/gpu_run.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: experimental/libzync-bench/signet/gpu_r4/gpu_run2.sh picks the first GLOBAL mapping, not 100.65 or a later one",
    "H1: the GLOBAL 22 mapping wins whether the 100.65 entry comes first or last",
    "H3(r3): rp_term a b c sends EVERY DELETE first, then confirms",
    "L(r3): a Cloudflare page is FATAL",
    "L1(r2): a 500 on create looks the pod up BY NAME and returns it",
    "L1: a 429 listing makes pod.sh status FAIL, not print 'no pods'",
    "L2: a pod deployed at cost 0 whose price turns out $0.3/h against a $0.50 ceiling is booting before it boots",
    "L2: a pod deployed at cost 0 whose price turns out $0.9/h against a $0.50 ceiling is terminated before it boots",
    "L3(r2): GET /v2/pods answering 200 without `pods`/`pagination` RAISES",
    "M1(r2): a 429's Retry-After",
    "M1(r2): every DELETE goes out FIRST, then one listing confirms them all",
    "M1(r3): a pod over the ceiling, again and again, latches after RUNPOD_REFUSALS_MAX",
    "M1: HTTP 401 on the first type is a refusal even when another type answered 400",
    "M1: HTTP 402 on the first type is a refusal even when another type answered 400",
    "M1: HTTP 422 on the first type is a refusal even when another type answered 400",
    "M2(r2): the next start finds the earlier run's unconfirmed pod, cannot stop it, and REFUSES to rent",
    "M2(r3): the lost create's NEW pod is found, never the earlier run's pod of the same name",
    "M2(r3): two live pods under one lost name: the first is kept, the other RELEASED, a TERMINATED one ignored",
    "M2: a 429 on deploy is NOT a refusal -- it never moves the latch",
    "M2: and it BACKS OFF while RunPod refuses",
    "M2: twenty 429'd DELETEs do not use up terminate_confirmed's 6 tries -- the pod is still released",
    "M2: wait_for_ssh survives 429s from ports_of and still brings the card up",
    "M3(r2): a pod RunPod still lists after terminate stays LIVE, state unconfirmed",
    "M3(r2): and every poll is PACED for the 120/min sustained quota",
    "M3(r2): and no replacement is rented on top of it at --max-pods 1",
    "M3(r2): its spend keeps counting",
    "M3(r3): a lost answer whose follow-up listing FAILS exits 4",
    "M3: every rp/gqlbal curl has --max-time",
    "M4(r2): rp_term retries until RunPod answers 404",
    "M4(r3): the pod a lost create made under the run's own prefix is found and released at once",
    "M4(r3): when it cannot be confirmed the run STOPS",
    "RunPod's own 403",
    "a pod whose listing has no runtime still gets its SSH and boots",
    "ssh_of asks GET /v2/pods/{id} and takes the PUBLIC 22 mapping, not a 10.x one",
    "two 429s are waited out for Retry-After and the create then lands",
    "with no runtime ports, RunPod's own ssh.direct is used",
}
if CONTROL:
    caught = {e for e in EXPECTED_CONTROL if any(f.startswith(e) for f in fails)}
    missed = EXPECTED_CONTROL - caught
    extra = [f for f in fails if not any(f.startswith(e) for e in EXPECTED_CONTROL)]
    if missed or extra:
        print("CONTROL FAILED")
        for m in sorted(missed):
            print(f"  not caught: {m}")
        for x in extra:
            print(f"  unexpected failure: {x}")
        sys.exit(1)
    print(f"CONTROL OK — all {len(EXPECTED_CONTROL)} expected failures appeared, and nothing else failed")
    sys.exit(0)
if fails:
    print(f"⛔ {len(fails)} check(s) FAILED")
    sys.exit(1)
print("the RunPod client speaks REST v2 and keeps its GraphQL-era behaviour")
