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


if CONTROL:
    # ⚠ THE CONTROLS MUTATE THE SHIPPED CLIENT, one v2 difference each.
    sb.NO_CAPACITY_STATUS = None                      # a 400 read as a refusal, as a naive port would
    sb._public_ip = lambda ip: True                   # isIpPublic dropped: a 10.x entry is taken for SSH
    sb.RATE_LIMIT_RETRIES = 0                         # a 429 surfaces as a refusal
    sb.RunPod.ssh_of = lambda self, pod_id: None      # only the listing is read for SSH


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
                out = b"" if obj is None else json.dumps(obj).encode()
                self.send_response(code)
                for k, v in hdrs:
                    self.send_header(k, v)
                if out:
                    self.send_header("Content-Type", "application/problem+json" if code >= 400 else "application/json")
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
            self.pods[pid] = {"name": body["name"], "gpu": body["gpu"]["id"], "cost": 0.74, "dc": None,
                              "ports": [{"private": 22, "public": 40022, "type": "tcp", "ip": PUBLIC_IP}]}
            return 201, self.pod_obj(pid), ()
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
      "a refusal on one type and a capacity miss on the next is an ANSWER (None), as with GraphQL")

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
EXPECTED_CONTROL = {
    "400 for every type is 'no capacity' (None), NOT a refusal",
    "a refusal on one type and a capacity miss on the next is an ANSWER",
    "two 429s are waited out for Retry-After",
    "ssh_of asks GET /v2/pods/{id} and takes the PUBLIC 22 mapping",
    "ports_of keeps exactly the public mappings",
    "when the listing does carry runtime, its public mapping is used directly",
    "a pod whose listing has no runtime still gets its SSH and boots",
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
    print(f"CONTROL OK — all {len(EXPECTED_CONTROL)} wrong v2 mappings were caught")
    sys.exit(0)
if fails:
    print(f"⛔ {len(fails)} check(s) FAILED")
    sys.exit(1)
print("the RunPod client speaks REST v2 and keeps its GraphQL-era behaviour")
