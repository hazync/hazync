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
The final section sweeps the repo: no live code may still call GraphQL. experimental/libzync-bench/ is exempt: those
are historical run scripts, kept on GraphQL and marked "port before reuse".

  python3 test_runpod_v2.py            # must PASS
  python3 test_runpod_v2.py --control  # the v2 mapping put back wrong; the named checks MUST fail
"""
import http.server
import subprocess
import time
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

_TM = sb.terminate_many           # the real one, for the tests that must reach it whatever the controls replace

if CONTROL:
    # ⚠ THE CONTROLS MUTATE THE SHIPPED CLIENT, one v2 difference each.
    sb.NO_CAPACITY_STATUS = None                      # a 400 read as a refusal, as a naive port would
    sb._public_ip = lambda ip: not ipaddress.ip_address(str(ip)).is_private   # H1: "not private" is not "public"
    sb.RATE_LIMIT_RETRIES = 0                         # a 429 surfaces as a refusal
    sb.RunPod.ssh_of = lambda self, pod_id: None      # only the listing is read for SSH
    sb.REQUEST_REFUSALS = ()                          # M1: a 401/402/422 hidden behind another type's 400
    sb.CAPACITY_LATCH_S = 0                           # H1(r2): a drought latches on the first miss
    sb.Bot._sweep_strays = lambda self, c=None: None  # r4: a lost create's pod is never swept during the run
    sb._ACCEPT_UNACKED_ABSENCE = True                 # r6 M-1: one listing's absence taken for "gone"
    sb.release_until_clean = (lambda api, ids, match=None, skip=None, sleep=None, clock=None, budget_s=None,  # r6: one pass
                              wait_s=5.0, gap_s=5.0, on_done=None:
                              (lambda d, l: (on_done and d and on_done(d)) or (d, l, not l))(
                                  *_TM(api, list(ids), lambda x: None, 1, 0, 0)))
    sb.Bot._landed = lambda self, p: False            # r5 H1: a working pod does not end the drought
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
        self.late = 0                                    # a lost create's pod is missing from this many listings
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
        return {"id": pid, "name": p["name"], "status": p.get("status", "RUNNING"), "cost": p["cost"], "cloud": "SECURE",
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
            # RunPod's real create answer: PROVISIONING, no runtime yet, no data centre.
            obj = dict(self.pod_obj(pid, runtime=False), status="PROVISIONING", dataCenterId=None)
            self.pods[pid]["cost"] = self.cost_after
            if self.create_then_fail > 0:
                self.create_then_fail -= 1
                self.pods[pid]["hidden"] = self.late
                return self.problem(500, "upstream timeout")
            return 201, obj, ()
        if method == "GET" and path == "/v2/pods" and self.bad_listing:
            return 200, {"message": "ok"}, ()
        if method == "GET" and path == "/v2/pods":
            ids = []
            for i in sorted(self.pods):
                if self.pods[i].get("hidden", 0) > 0:      # not listed yet (eventual consistency)
                    self.pods[i]["hidden"] -= 1
                    continue
                ids.append(i)
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

ROOT = pathlib.Path(HERE).parent


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

# ── r4. ONE drought rule: N hours of attempts with nothing landing, whatever the reason, latches ─────────
print("== drought(r4): the capacity streak ==")
with tempfile.TemporaryDirectory() as home:
    t0 = 1_000_000.0
    seen = {h / 2: sb.capacity_note(home, t0 + h * 1800) for h in range(0, 15)}    # an attempt every 30 min
    first = min((h for h, v in seen.items() if v), default=None)
    check(first == 6.0, f"drought(r4): attempts every 30 min with nothing landing latch at 6 h, not before ({first} h)")
    sb.capacity_note(home, t0)
    check(sb.capacity_note(home, t0 + 5 * 3600) is False,
          "drought(r4): a gap of over 2 h without an attempt ends the streak (nothing was waiting)")
    for junk in ('{"first": "x"}', "not json", '[]', '{"first": 5, "last": 1}', '{"first": 1e30, "last": 1e30}'):
        open(os.path.join(home, sb.CAPACITY_STATE), "w").write(junk)
        try:
            v = sb.capacity_note(home, t0)
        except Exception as e:      # noqa: BLE001
            v = e
        check(v is False, f"drought(r4): a malformed capacity.json ({junk!r}) starts afresh, never raises ({v!r})")


D1 = "There are no instances currently available"


class LimitedAPI:
    last_misses = []

    def deploy(self, name, pub):
        raise sb.RunPodError("HTTP 429 Too Many Requests", status=429)


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
      f"drought(r4): three looks of a normal drought (LOW stock, all-400) neither latch nor page "
      f"({bot.stop_reason}, {bot._refusals})")
check(any(D1 in m for m in logs), "H1(r2): and the log says RunPod's own words for the miss")
with tempfile.TemporaryDirectory() as home:
    now = time.time()
    json.dump({"first": now - 5 * 3600, "last": now - 300}, open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(MissAPI([D1, D1]), 1, home=home)
    check(bot.stop_reason is None, f"drought(r4): 5 h with nothing landing does not stop the bot yet ({bot.stop_reason})")
    json.dump({"first": now - 6.1 * 3600, "last": now - 300}, open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(MissAPI([D1, "another reason"]), 1, home=home)
    check(bot.stop_reason == "runpod" and not os.path.exists(os.path.join(home, sb.CAPACITY_STATE)),
          f"drought(r4): 6 h with nothing landing stops it as 'runpod' (exit 5), whatever the 400s said, and the "
          f"streak is cleared ({bot.stop_reason})")
    json.dump({"first": now - 6.1 * 3600, "last": now - 300}, open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    bot, logs = looks(LimitedAPI(), 1, home=home)
    check(bot.stop_reason == "runpod", f"drought(r4): a 429 counts toward the drought too ({bot.stop_reason})")

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
        bot._terminate(c, p1, "test")
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

# ── r4. a create whose answer was lost is found by the run's PREFIX SWEEP, however late it appears ──────
print("== lost create (r4): the prefix sweep ==")
f = FakeV2()
f.create_then_fail, f.late = 1, 1
try:
    got = client(f).deploy("hz-sponsor-l1", PUB, gpu_types=(R4090,))
    raised = None
except sb.RunPodError as e:
    got, raised = None, e
check(raised is not None and len(f.pods) == 1 and not client(f).pods(),
      f"r4: a create that made the pod but answered 500 is just 'no answer'; the pod is NOT listed yet ({raised})")


class SweepAPI(sb.RunPod):
    pass


f = FakeV2()
f.create_then_fail, f.late = 1, 2                     # missing from the first TWO listings
api = sb.RunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
with tempfile.TemporaryDirectory() as d:
    logs = []
    bot = sb.Bot(os.path.join(d, "bot.db"), api, RL(), max_pods=2, max_usd=10, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=logs.append, sleep=lambda x: None, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        bot._launch(c, bot.clock())                  # type 1's answer is lost (its pod exists); type 2 lands
        lost = [i for i in f.pods if i not in {p_.id for p_ in bot.pods}]
        for _ in range(4):                           # four looks: the lost pod appears on the third listing
            bot._sweep_strays(None)
    finally:
        sb.pending_work = real
        c.close()
check(len(lost) == 1 and lost[0] not in f.pods and len(f.pods) == 1 and any("not tracked" in m for m in logs),
      f"r4: the bot's per-look prefix sweep releases the lost create's pod once it appears, LATE, and keeps the one "
      f"it tracks ({lost}, left {list(f.pods)})")

# ── r4. pods over the price ceiling, look after look, are a drought ──────────────────────────────────
print("== drought(r4): over the price ceiling ==")


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
clk = Clock()
clk.t = 1_000_000.0
with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), da, RL(), max_pods=2, max_usd=100, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0, clock=clk)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        for _ in range(20):
            if bot.stop_reason:
                break
            bot._launch(c, bot.clock())
            clk.t += 1800
    finally:
        sb.pending_work = real
    c.close()
check(bot.stop_reason == "runpod" and 12 <= da.n <= 14,
      f"drought(r4): every pod back over the price ceiling for 6 h stops the bot as 'runpod' ({da.n} pods, "
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

# ── r4. tip_smoke: the teardown sweeps the run's prefix by listing, so a LATE lost create goes too ──────
print("== lost create (r4): tip_smoke's prefix sweep ==")
if CONTROL:
    _real_release = tip_smoke.release_recorded
    tip_smoke.release_recorded = lambda api, rp, cr, un, prefix=None, **kw: _real_release(api, rp, cr, un, None, **kw)
with tempfile.TemporaryDirectory() as d:
    f = FakeV2()
    f.pods["old1"] = {"name": "hz-smoke-1", "gpu": A40, "cost": 1}          # another run's pod, old naming
    api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
    pre = tip_smoke.run_prefix(d)
    f.create_then_fail, f.late = 1, 1
    try:
        api.deploy_listening(f"{pre}1", PUB, gpu_types=(A40,))
    except sb.RunPodError:
        pass
    kept = api.deploy_listening(f"{pre}2", PUB, gpu_types=(A40,))
    left = tip_smoke.release_recorded(api, os.path.join(d, "rented.json"), [kept], [], prefix=pre,
                                      keep_ids=[kept["id"]], sleep=(_c := Clock()).sleep, clock=_c)
    names = sorted(x["name"] for x in f.pods.values())
check(pre.startswith("hz-smoke-") and len(pre) > len("hz-smoke-") + 1 and tip_smoke.run_prefix(tempfile.mkdtemp(), now=1) != pre,
      f"r4: the run prefix is unique per run ({pre})")
check(names == sorted(["hz-smoke-1", f"{pre}2"]),
      f"r4: --keep keeps the fleet, the teardown still releases the LATE lost create under the run's prefix, and "
      f"never another run's pod ({names})")

# ── r4. fold_rent: released by its unique NAME too ────────────────────────────────────────────────────
print("== lost create (r4): fold_rent ==")
import fold_rent   # noqa: E402
if CONTROL:
    fold_rent.release_fold = lambda api, name, pod_id, **kw: sb.terminate_many(api, [pod_id] if pod_id else [])[1]
f = FakeV2()
f.create_then_fail, f.late = 1, 0
api = sb.RunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
try:
    api.deploy("hazync-fold-123", PUB, gpu_types=(A40,))
except sb.RunPodError:
    pass
fold_rent.release_fold(api, "hazync-fold-123", None, sleep=(_c := Clock()).sleep, clock=_c)
check(not f.pods, f"r4: fold_rent's teardown releases a lost create by its unique name, with no pod id ({list(f.pods)})")

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

    def _old_release(api, rented_path, created, unreleased, prefix=None, keep_ids=(), **kw):   # C1(r2): `created` only
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
        tip_smoke.rent_uniform(ra, A40, 5, PUB, rp_, created, unreleased, "hz-smoke-t-")
        raised = None
    except sb.RunPodError as e:
        raised = e
    check(raised is not None and sorted(p["id"] for p in created) == ["r1", "r2", "r3"],
          f"C1(r2): 3 rented then a 402: all 3 are handed to the tracked set ({[p['id'] for p in created]})")
    left = tip_smoke.release_recorded(ra, rp_, [], [], sleep=(_c := Clock()).sleep, clock=_c)
    check(left == [] and not ra.live and not os.path.exists(rp_),
          f"C1(r2): the teardown releases them FROM THE RECORD even when `created` is empty, then drops the record "
          f"({left}, live {sorted(ra.live)})")
with tempfile.TemporaryDirectory() as d:
    rp_ = os.path.join(d, "rented.json")
    ra = RentAPI(ok=3, then="miss")
    created, unreleased = [], []
    got = tip_smoke.rent_uniform(ra, A40, 5, PUB, rp_, created, unreleased, "hz-smoke-t-")
    check(len(got) == 3, f"C1(r2): 3 rented then a 400 (a miss) returns the 3 -- no exception, nothing lost ({len(got)})")
    ra.listing_fails = True
    left = tip_smoke.release_recorded(ra, rp_, [], [], sleep=(_c := Clock()).sleep, clock=_c)
    check(left is None and os.path.exists(rp_),
          "C1(r2): when the account cannot be listed the record is KEPT (never 'account check: clean')")

# ── r5 H1. a partial grant is not a drought: one working pod, 10 h of misses for the rest, never latches ────
print("== r5: partial grant ==")


class PartialAPI:
    last_misses = []

    def __init__(self):
        self.n = 0

    def deploy(self, name, pub):
        self.n += 1
        if self.n == 1:
            return {"id": "p1", "name": name, "gpu_type": A40, "cost_per_hr": 0.59}
        return None

    def pods(self):
        return [{"id": "p1", "name": "x", "status": "RUNNING", "cost_per_hr": 0.59}]

    def terminate(self, pid):
        pass


clk = Clock()
clk.t = 1_000_000.0
with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as home:
    bot = sb.Bot(os.path.join(d, "bot.db"), PartialAPI(), RL(), max_pods=4, max_usd=500, max_usd_per_hour=10,
                 capacity_backoff_s=0, log=lambda m: None, clock=clk)
    bot.state_home = home
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [(1, h) for h in range(200)]
    try:
        for i in range(40):                       # 10 h, a look every 15 min
            bot._launch(c, clk())
            if i == 0:
                bot.pods[0].state = "working"
            if bot.stop_reason:
                break
            clk.t += 900
    finally:
        sb.pending_work = real
        c.close()
check(bot.stop_reason is None, f"r5 H1: one pod working, the other three never granted for 10 h: the bot keeps "
      f"working, no drought stop ({bot.stop_reason})")

# ── r5 M3. a pod at cost 0 whose listing shows it over the ceiling: terminated at once, the drought not cleared ──
print("== r5: a price learnt from the listing ==")


class ZeroThenDear:
    last_misses = []

    def __init__(self):
        self.gone = set()

    def deploy(self, name, pub):
        self.name = name
        return {"id": "z1", "name": name, "gpu_type": A40, "cost_per_hr": 0}

    def pods(self):
        return [{"id": "z1", "name": self.name, "status": "RUNNING", "cost_per_hr": 3.0}] if "z1" not in self.gone else []

    def terminate(self, pid):
        self.gone.add(pid)

    def pod(self, pid):
        return None


with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as home:
    zd = ZeroThenDear()
    bot = sb.Bot(os.path.join(d, "bot.db"), zd, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    bot.state_home = home
    t_ = bot.clock()
    json.dump({"first": t_ - 3600, "last": t_ - 60}, open(os.path.join(home, sb.CAPACITY_STATE), "w"))
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        bot._launch(c, t_)
        streak_after_deploy = os.path.exists(os.path.join(home, sb.CAPACITY_STATE))
        bot._sweep_strays(c)
    finally:
        sb.pending_work = real
        c.close()
    st = json.load(open(os.path.join(home, sb.CAPACITY_STATE))) if os.path.exists(os.path.join(home, sb.CAPACITY_STATE)) else {}
check(streak_after_deploy, "r5 M3: a pod that lands at cost 0 (price unknown) does not clear the drought streak")
check("z1" in zd.gone and not bot.pods[0].live,
      "r5 M3: once the listing shows it at $3.00/h over the $1.00 ceiling it is terminated -- without waiting for SSH")
check(st.get("first") == t_ - 3600, f"r5 M3: and the streak goes on from where it was ({st})")

# ── r5 H2. tip_smoke refuses to start over a rundir whose last run still has pods ─────────────────────────────
print("== r5: tip_smoke rerun guard ==")
if CONTROL:
    tip_smoke.refuse_rerun = lambda api, rundir, rented_path: None
with tempfile.TemporaryDirectory() as d:
    f = FakeV2()
    api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
    old = tip_smoke.run_prefix(d)
    f.pods["leak1"] = {"name": f"{old}3", "gpu": A40, "cost": 1}
    try:
        tip_smoke.refuse_rerun(api, d, os.path.join(d, "rented.json"))
        r1 = None
    except SystemExit as e:
        r1 = str(e)
    del f.pods["leak1"]
    f.pods["leak2"] = {"name": "hz-smoke-adopted-1", "gpu": A40, "cost": 1}
    json.dump([{"id": "leak2", "name": "hz-smoke-adopted-1"}], open(os.path.join(d, "rented.json"), "w"))
    try:
        tip_smoke.refuse_rerun(api, d, os.path.join(d, "rented.json"))
        r2 = None
    except SystemExit as e:
        r2 = str(e)
    del f.pods["leak2"]
    try:
        tip_smoke.refuse_rerun(api, d, os.path.join(d, "rented.json"))
        r3 = None
    except SystemExit as e:
        r3 = str(e)
check(r1 and "refusing" in r1, f"r5 H2: a pod with the last run's prefix still listed refuses a new run ({r1})")
check(r2 and "refusing" in r2, f"r5 H2: so does a still-listed rented.json id ({r2})")
check(r3 is None, f"r5 H2: and nothing listed lets the run start ({r3})")

# ── r6 M-1. absence is believed only after an ANSWERED delete; a written-off pod still listed is revived ─────────
print("== r6: confirmations ==")


class OmitAPI:
    """DELETE errors (500) while the listing happens to omit the pod."""
    def __init__(self):
        self.deletes = 0

    def terminate(self, pid):
        self.deletes += 1
        raise sb.RunPodError("HTTP 500", status=500)

    def pods(self):
        return []


clk = Clock()
oa = OmitAPI()
done, left = _TM(oa, ["x1"], sleep=clk.sleep, clock=clk, tries=2, wait_s=1)
check(left == ["x1"] and not done,
      f"r6 M-1: a DELETE that errored is not 'gone' because one listing omits the pod ({done}, {left})")


class LagAPI:
    last_misses = []

    def __init__(self):
        self.listed, self.fail_delete, self.deletes = False, True, 0

    def deploy(self, name, pub):
        self.name = name
        return {"id": "d1", "name": name, "gpu_type": A40, "cost_per_hr": 3.0}

    def terminate(self, pid):
        self.deletes += 1
        if self.fail_delete:
            raise sb.RunPodError("HTTP 500", status=500)
        self.listed = False

    def pods(self):
        return [{"id": "d1", "name": self.name, "status": "RUNNING", "cost_per_hr": 3.0}] if self.listed else []

    def pod(self, pid):
        return None


if CONTROL:
    sb.Bot._sweep_strays = lambda self, c=None: None
clk = Clock()
with tempfile.TemporaryDirectory() as d:
    la = LagAPI()
    bot = sb.Bot(os.path.join(d, "bot.db"), la, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5, capacity_backoff_s=0,
                 log=lambda m: None, clock=clk, sleep=clk.sleep, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        bot._launch(c, clk())                           # over the ceiling: DELETE errors, the listing lags
        written_off = bot.pods[0].terminated is not None
        la.listed, la.fail_delete = True, False         # RunPod now lists it, and a DELETE would work
        for _ in range(3):
            bot._retry_unconfirmed(c)
            bot._sweep_strays(c)
            clk.t += 30
    finally:
        sb.pending_work = real
        c.close()
check(not la.listed, f"r6 M-1: a pod RunPod still lists after it was taken for gone is swept again and released "
      f"(written off at first: {written_off}, still listed: {la.listed})")

clk = Clock()
with tempfile.TemporaryDirectory() as d:
    la = LagAPI()
    la.fail_delete = False                              # the DELETE is answered, the listing lags: written off
    bot = sb.Bot(os.path.join(d, "bot.db"), la, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5, capacity_backoff_s=0,
                 log=lambda m: None, clock=clk, sleep=clk.sleep, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        bot._launch(c, clk())
        written_off = bot.pods[0].terminated is not None
        la.listed = True                                # ...and then RunPod lists it after all
        d0 = la.deletes
        for _ in range(3):
            bot._sweep_strays(c)
            bot._retry_unconfirmed(c)
            clk.t += 30
    finally:
        sb.pending_work = real
        c.close()
check(written_off and la.deletes > d0 and not la.listed,
      f"r6 M-1: a pod written off as gone that RunPod lists again is revived and released "
      f"(written off {written_off}, {la.deletes - d0} more DELETEs, still listed {la.listed})")

# ── r6. a pod listed as TERMINATED is gone, and never a stray ───────────────────────────────────────────────
f = FakeV2()
api = sb.RunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), api, RL(), max_pods=1, max_usd=10, max_usd_per_hour=5,
                 log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    f.pods["t1"] = {"name": bot.prefix + "9", "gpu": A40, "cost": 1, "status": "TERMINATED"}
    bot._sweep_strays(None)
check(not any(r[0] == "DELETE" for r in f.reqs), "r6: a pod RunPod lists as TERMINATED is not taken for a stray")

# ── r6. a stray that will not go is tracked: spend, --max-pods, retried ─────────────────────────────────────
class StuckStray:
    last_misses = []

    def __init__(self, name):
        self.name, self.n = name, 0

    def deploy(self, name, pub):
        self.n += 1
        return {"id": f"n{self.n}", "name": name, "gpu_type": A40, "cost_per_hr": 0.4}

    def terminate(self, pid):
        pass

    def pods(self):
        return [{"id": "s9", "name": self.name, "status": "RUNNING", "cost_per_hr": 0.5}]


with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), None, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 capacity_backoff_s=0, log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    bot.api = StuckStray(bot.prefix + "7")
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    bot._sweep_strays(c)
    real = sb.pending_work
    sb.pending_work = lambda *a, **k: [1]
    try:
        bot._launch(c, bot.clock())
    finally:
        sb.pending_work = real
    row = c.execute("SELECT terminated_at FROM sponsor_pods WHERE pod_id='s9'").fetchone()
    c.close()
check([p.id for p in bot._live()] == ["s9"] and bot.pods[0].state == "unconfirmed" and "s9" in bot.unconfirmed
      and row is not None and row[0] is None and bot.api.n == 0,
      f"r6: a stray that will not terminate is tracked (live, unconfirmed, a row for the next start) and holds its "
      f"--max-pods slot: nothing else is rented ({[p.id for p in bot._live()]}, {bot.api.n} deploys)")

# ── r6. terminate_all: a pod still listed after the first pass is confirmed on the second ───────────────────
class TwoPass:
    def __init__(self):
        self.listings, self.deleted = 0, False

    def terminate(self, pid):
        self.deleted = True

    def pods(self):
        self.listings += 1
        return [{"id": "p1", "name": "x", "status": "RUNNING"}] if self.listings <= 3 else []


clk = Clock()
with tempfile.TemporaryDirectory() as d:
    tp = TwoPass()
    bot = sb.Bot(os.path.join(d, "bot.db"), tp, RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 log=lambda m: None, clock=clk, sleep=clk.sleep, confirm_wait_s=1)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    p_ = sb.Pod({"id": "p1", "name": bot.prefix + "1", "gpu_type": A40, "cost_per_hr": 0.5}, clk())
    bot.pods.append(p_)
    c.execute("INSERT INTO sponsor_pods(pod_id, name, created_at) VALUES('p1','n',?)", (clk(),))
    c.commit()
    bot.terminate_all(c, "test")
    row = c.execute("SELECT terminated_at FROM sponsor_pods WHERE pod_id='p1'").fetchone()
    c.close()
check(p_.state == "terminated" and not bot.unconfirmed and row[0] is not None,
      f"r6: terminate_all confirms a pod that RunPod still listed on the first pass ({p_.state}, {bot.unconfirmed})")

# ── r6. main(): no SPONSOR_BOT_HOME refuses --live before RunPod is touched; the sweep runs before prepare() ─────
print("== r6: main() ordering ==")
order = []
saved = (sb.Coord.from_env, sb.RunPod.from_env, sb.SshRunner.from_env, sb.Bot.release_leftovers, sb.connect_path,
         os.environ.get("SPONSOR_BOT_HOME"))
with tempfile.TemporaryDirectory() as d:
    sb.Coord.from_env = classmethod(lambda cls: object())
    sb.connect_path = lambda: os.path.join(d, "bot.db")
    sb.RunPod.from_env = classmethod(lambda cls: order.append("runpod") or StuckStray("x"))

    class _R:
        def prepare(self):
            order.append("prepare")
            raise SystemExit("stop here")
    sb.SshRunner.from_env = classmethod(lambda cls: _R())
    sb.Bot.release_leftovers = lambda self: order.append("sweep")
    argv = ["run", "--live", "--max-pods", "1", "--max-usd", "1", "--max-usd-per-hour", "1"]
    os.environ.pop("SPONSOR_BOT_HOME", None)
    try:
        sb.main(argv)
        r1 = None
    except SystemExit as e:
        r1 = str(e)
    o1, order[:] = list(order), []
    os.environ["SPONSOR_BOT_HOME"] = d
    try:
        sb.main(argv)
    except SystemExit:
        pass
    o2 = list(order)
(sb.Coord.from_env, sb.RunPod.from_env, sb.SshRunner.from_env, sb.Bot.release_leftovers, sb.connect_path) = saved[:5]
if saved[5] is None:
    os.environ.pop("SPONSOR_BOT_HOME", None)
else:
    os.environ["SPONSOR_BOT_HOME"] = saved[5]
check(r1 and "SPONSOR_BOT_HOME" in r1 and o1 == [],
      f"r6 M-2: without SPONSOR_BOT_HOME --live is refused before RunPod is touched ({r1}, {o1})")
check(o2[:3] == ["runpod", "sweep", "prepare"], f"r6 M-1: the start sweep runs before SshRunner.prepare() ({o2})")

# ── r6. fold_rent's release lists again: a lost create that appears late goes too ───────────────────────────
f = FakeV2()
f.create_then_fail, f.late = 1, 1
api = sb.RunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
try:
    api.deploy("hazync-fold-late", PUB, gpu_types=(A40,))
except sb.RunPodError:
    pass
fold_rent.release_fold(api, "hazync-fold-late", None, sleep=(_c := Clock()).sleep, clock=_c)
check(not f.pods, f"r6: fold_rent's release catches a lost create that only appears on the second listing ({list(f.pods)})")

# ── r7 H1. --adopt never deletes the adopted fleet: gate drops, failed adoptions, someone else's rundir ─────────
print("== r7: adopt ==")
import argparse as _ap   # noqa: E402
if CONTROL:
    def _old_teardown(api, a, rented_path, created, unreleased, rpre, adopt_keep, sleep=None, clock=None):
        keep = [p["id"] for p in created] if (a.keep or (a.adopt and not a.release_adopted)) else []
        return tip_smoke.release_recorded(api, rented_path, created, unreleased, prefix=rpre, keep_ids=keep,
                                          sleep=sleep, clock=clock)
    tip_smoke.teardown_release = _old_teardown

    def _old_guard(api, a):
        if not a.cleanup and not a.adopt:
            tip_smoke.refuse_rerun(api, a.rundir, os.path.join(a.rundir, "rented.json"))
        return bool(a.adopt)
    tip_smoke.rerun_guard = _old_guard
for case in ("gate drop", "adoption failed"):
    with tempfile.TemporaryDirectory() as d:
        f = FakeV2()
        api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
        pre = tip_smoke.run_prefix(d)
        rec = [{"id": f"a{i}", "name": f"{pre}{i}"} for i in (1, 2, 3)]
        for r_ in rec:
            f.pods[r_["id"]] = {"name": r_["name"], "gpu": A40, "cost": 1}
        rp_ = os.path.join(d, "rented.json")
        json.dump(rec, open(rp_, "w"))
        a = _ap.Namespace(keep=False, adopt=rp_, release_adopted=False, cleanup=False, rundir=d)
        keep = tip_smoke.adopted_ids(rp_)
        created = [r_ for r_ in rec if r_["id"] != "a3"] if case == "gate drop" else []
        if case == "gate drop":
            json.dump(created, open(rp_, "w"))           # the gate drop rewrote rented.json without a3
        tip_smoke.teardown_release(api, a, rp_, created, [], pre, keep, sleep=(_c := Clock()).sleep, clock=_c)
        survivors = sorted(f.pods)
    check(survivors == ["a1", "a2", "a3"],
          f"r7 H1: --adopt ({case}): every pod of the adopted record survives the teardown ({survivors})")
with tempfile.TemporaryDirectory() as other, tempfile.TemporaryDirectory() as R:
    f = FakeV2()
    api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
    kept_pre = tip_smoke.run_prefix(R)                 # R holds a --keep fleet
    f.pods["k1"] = {"name": f"{kept_pre}1", "gpu": A40, "cost": 1}
    json.dump([{"id": "o1", "name": "x"}], open(os.path.join(other, "rented.json"), "w"))
    a = _ap.Namespace(adopt=os.path.join(other, "rented.json"), cleanup=False, rundir=R)
    try:
        tip_smoke.rerun_guard(api, a)
        refused = None
    except SystemExit as e:
        refused = str(e)
check(refused and "refusing" in refused,
      f"r7 H1: --adopt of another rundir's record into a rundir that holds a kept fleet is refused ({refused})")

# ── r8 M1. a pod unconfirmed for over an hour does NOT stop the run: it is retried and said loudly ─────────────
if CONTROL:
    sb.UNCONFIRMED_WARN_S = 10 ** 9


class Never:
    def terminate(self, pid):
        pass

    def pods(self):
        return [{"id": "u1", "name": "x", "status": "RUNNING"}]


clk = Clock()
with tempfile.TemporaryDirectory() as d:
    logs = []
    bot = sb.Bot(os.path.join(d, "bot.db"), Never(), RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 log=logs.append, clock=clk, sleep=clk.sleep, confirm_wait_s=0)
    pu = sb.Pod({"id": "u1", "name": bot.prefix + "1", "gpu_type": A40, "cost_per_hr": 0.5}, clk())
    pu.state, pu.unconfirmed_since = "unconfirmed", clk() - 3700
    bot.pods.append(pu)
    bot._retry_unconfirmed(None)
    bot._retry_unconfirmed(None)
check(bot.stop_reason is None and pu.live and sum("not been confirmed terminated" in m for m in logs) == 1,
      f"r8 M1: a pod unconfirmed for over an hour is retried, not a stop, and said loudly ONCE per hour "
      f"({bot.stop_reason}, {sum('not been confirmed terminated' in m for m in logs)} warnings)")

# ── r7 L4. the start sweep closes a row only after an answered DELETE ─────────────────────────────────────────
class Unlisted:
    def pods(self):
        return []

    def terminate(self, pid):
        raise sb.RunPodError("HTTP 500", status=500)


with tempfile.TemporaryDirectory() as d:
    bot = sb.Bot(os.path.join(d, "bot.db"), Unlisted(), RL(), max_pods=1, max_usd=100, max_usd_per_hour=5,
                 log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0)
    c = sb.connect(bot.db_path)
    sb.ensure_tables(c)
    c.execute("INSERT INTO sponsor_pods(pod_id, name, created_at) VALUES('q1','hz-sponsor-1-1',1)")
    c.commit()
    if CONTROL:                                  # the old sweep: an unlisted row is closed without a DELETE
        c.execute("UPDATE sponsor_pods SET terminated_at=1 WHERE pod_id='q1'")
    else:
        bot._sweep_leftovers(c)
    row = c.execute("SELECT terminated_at FROM sponsor_pods WHERE pod_id='q1'").fetchone()
    c.close()
check(row[0] is None, "r7 L4: a pod the listing omits keeps its row open when its DELETE errors (not proof it is gone)")

# ── r8 H1. two consecutive --adopt runs, through main()'s record wiring: a pod LEFT RUNNING is never deleted ──
print("== r8: adopt twice ==")
if CONTROL:
    def _old_release(self, api, p, why, adopted):      # the record forgets a pod it left running
        if not adopted:
            sb.terminate_confirmed(api, p["id"])
    tip_smoke.FleetRecord.release = _old_release


class _Card:
    def __init__(self, cid):
        self.cid = cid


def adopt_run(api, d, drop):
    """One --adopt run of d's own record, wired as main() wires it: the snapshot at start, rerun_guard, the prefix,
    FleetRecord + _drop_cards at a gate, teardown_release at the end. `drop`: card names a gate drops."""
    rp_ = os.path.join(d, "rented.json")
    a = _ap.Namespace(keep=False, adopt=rp_, release_adopted=False, cleanup=False, rundir=d)
    keep = tip_smoke.adopted_ids(a.adopt)
    home = tip_smoke.rerun_guard(api, a)
    rpre = (home and tip_smoke.read_run_prefix(d)) or tip_smoke.run_prefix(d)
    unreleased = []
    rec = tip_smoke.FleetRecord(rp_, unreleased)
    try:
        created, missing = tip_smoke.adopt_fleet(api, a.adopt)
        order = [_Card(p["name"]) for p in created]
        order, created = tip_smoke._drop_cards(order, created, drop, "a wrong --repo failed the staging gate",
                                               release=lambda p: rec.release(api, p, "gate", adopted=True),
                                               record=rec.write)
    except SystemExit:
        created = []
    finally:
        tip_smoke.teardown_release(api, a, rp_, created, unreleased, rpre, keep, sleep=(_c := Clock()).sleep, clock=_c)


for scenario, first_drop in (("A: one card dropped at a gate", "one"), ("B: every card dropped (#551)", "all")):
    with tempfile.TemporaryDirectory() as d:
        f = FakeV2()
        api = tip_smoke.SmokeRunPod("k-test", url=f.url, timeout=10, sleep=lambda x: None)
        pre = tip_smoke.run_prefix(d)
        rec0 = [{"id": f"a{i}", "name": f"{pre}{i}"} for i in (1, 2, 3, 4)]
        for r_ in rec0:
            f.pods[r_["id"]] = {"name": r_["name"], "gpu": A40, "cost": 1}
        json.dump(rec0, open(os.path.join(d, "rented.json"), "w"))
        names = [r_["name"] for r_ in rec0]
        adopt_run(api, d, names[3:] if first_drop == "one" else names)     # run 1
        adopt_run(api, d, [])                                                # run 2: adopt the same record again
        survivors = sorted(f.pods)
    check(survivors == ["a1", "a2", "a3", "a4"],
          f"r8 H1 ({scenario}): two consecutive --adopt runs never delete a pod that was LEFT RUNNING ({survivors})")

# ── r8 M2. the final pass DELETEs a stray that was never tried, once ──────────────────────────────────────────
class LastMinute:
    def __init__(self):
        self.deletes, self.live = [], {"late1"}

    def pods(self):
        return [{"id": i, "name": "pre-" + i, "status": "RUNNING"} for i in self.live]

    def terminate(self, pid):
        self.deletes.append(pid)
        self.live.discard(pid)


clk = Clock()
lm = LastMinute()
done, left, clean = sb.release_until_clean(lm, [], match=lambda n: n.startswith("pre-"), sleep=clk.sleep, clock=clk,
                                           budget_s=4, gap_s=5)
check(lm.deletes == ["late1"] and done == ["late1"] and left == [],
      f"r8 M2: a stray first seen on the last listing gets exactly one DELETE before the budget gives up ({lm.deletes})")

# ── r8. terminate_all: every DELETE before the coordinator is asked which blocks landed ─────────────────────────
events = []


class Recorder:
    def terminate(self, pid):
        events.append("DELETE")

    def pods(self):
        return []


real_cov = sb.covered
sb.covered = lambda coord, heights: events.append("books") or set()
try:
    with tempfile.TemporaryDirectory() as d:
        bot = sb.Bot(os.path.join(d, "bot.db"), Recorder(), RL(), max_pods=2, max_usd=100, max_usd_per_hour=5,
                     log=lambda m: None, sleep=lambda x: None, confirm_wait_s=0, coord=object())
        c = sb.connect(bot.db_path)
        sb.ensure_tables(c)
        for i in (1, 2):
            pp = sb.Pod({"id": f"b{i}", "name": bot.prefix + str(i), "gpu_type": A40, "cost_per_hr": 0.5}, bot.clock())
            pp.assigned = [{"sid": 1, "height": i, "row": i}]
            bot.pods.append(pp)
        bot.terminate_all(c, "test")
        c.close()
finally:
    sb.covered = real_cov
check(events and events.index("books") > max(i for i, e in enumerate(events) if e == "DELETE"),
      f"r8 L2: terminate_all sends every DELETE before it asks the coordinator about landed blocks ({events})")

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
    if p.relative_to(ROOT).parts[:2] == ("experimental", "libzync-bench"):
        continue            # historical run scripts: GraphQL on purpose, each marked "port before reuse"
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
    "H1(r2): every 400's detail is kept for the log and the streak",
    "H1: a pod with only a 100.65 address",
    "H1: and the same for 9110",
    "H1: the GLOBAL 22 mapping wins whether the 100.65 entry comes first or last",
    "L2: a pod deployed at cost 0 whose price turns out $0.3/h against a $0.50 ceiling is booting before it boots",
    "L2: a pod deployed at cost 0 whose price turns out $0.9/h against a $0.50 ceiling is terminated before it boots",
    "L3(r2): GET /v2/pods answering 200 without `pods`/`pagination` RAISES",
    "M1(r2): a 429's Retry-After",
    "M1(r2): every DELETE goes out FIRST, then one listing confirms them all",
    "M1: HTTP 401 on the first type is a refusal even when another type answered 400",
    "M1: HTTP 402 on the first type is a refusal even when another type answered 400",
    "M1: HTTP 422 on the first type is a refusal even when another type answered 400",
    "M2(r2): the next start finds the earlier run's unconfirmed pod, cannot stop it, and REFUSES to rent",
    "M2: a 429 on deploy is NOT a refusal -- it never moves the latch",
    "M2: and it BACKS OFF while RunPod refuses",
    "M2: twenty 429'd DELETEs do not use up terminate_confirmed's 6 tries -- the pod is still released",
    "M2: wait_for_ssh survives 429s from ports_of and still brings the card up",
    "M3(r2): a pod RunPod still lists after terminate stays LIVE, state unconfirmed",
    "M3(r2): and every poll is PACED for the 120/min sustained quota",
    "M3(r2): and no replacement is rented on top of it at --max-pods 1",
    "M3(r2): its spend keeps counting",
    "RunPod's own 403",
    "a pod whose listing has no runtime still gets its SSH and boots",
    "drought(r4): 5 h with nothing landing does not stop the bot yet",
    "drought(r4): a gap of over 2 h without an attempt ends the streak",
    "drought(r4): a malformed capacity.json",
    "drought(r4): attempts every 30 min with nothing landing latch at 6 h, not before",
    "drought(r4): every pod back over the price ceiling for 6 h stops the bot as 'runpod'",
    "drought(r4): three looks of a normal drought",
    "r4: --keep keeps the fleet, the teardown still releases the LATE lost create under the run's prefix, and never another run's pod",
    "r4: fold_rent's teardown releases a lost create by its unique name, with no pod id",
    "r4: the bot's per-look prefix sweep releases the lost create's pod once it appears, LATE, and keeps the one it tracks",
    "r5 H1: one pod working, the other three never granted for 10 h: the bot keeps working, no drought stop",
    "r5 H2: a pod with the last run's prefix still listed refuses a new run",
    "r5 H2: so does a still-listed rented.json id",
    "r5 M3: once the listing shows it at $3.00/h over the $1.00 ceiling it is terminated -- without waiting for SSH",
    "r6 M-1: a DELETE that errored is not 'gone' because one listing omits the pod",
    "r6 M-1: a pod RunPod still lists after it was taken for gone is swept again and released",
    "r6 M-1: a pod written off as gone that RunPod lists again is revived and released",
    "r6: a stray that will not terminate is tracked",
    "r6: fold_rent's release catches a lost create that only appears on the second listing",
    "r6: terminate_all confirms a pod that RunPod still listed on the first pass",
    "r7 H1: --adopt",
    "r7 H1: --adopt of another rundir's record into a rundir that holds a kept fleet is refused",
    "r7 L4: a pod the listing omits keeps its row open when its DELETE errors",
    "r8 H1",
    "r8 M1: a pod unconfirmed for over an hour is retried, not a stop, and said loudly ONCE per hour",
    "r8 M2: a stray first seen on the last listing gets exactly one DELETE before the budget gives up",
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
