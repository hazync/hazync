#!/usr/bin/env python3
"""rpm_es.py (eval-check-split run; rpm.py with the GPU listing and cloud as arguments)\nrpm.py <cmd> ...: the measurement run's RunPod control, from the LAPTOP only, through the board's own client
(coordinator/sponsor_bot.py RunPod; RUNPOD_API_KEY_FILE must point at the key file, which is read in-process and
never printed). Commands:
   balance                      clientBalance (the one GraphQL call; v2 has none; RunPod needs a User-Agent)
   list                         every pod: id name status cost/hr
   deploy <name> <pubkey> <gpu id> <SECURE|COMMUNITY>   one pod, 150 GB disk; prints "<id> <cost/hr>"
   ssh <id>                     "<ip> <port>" or nothing yet
   terminate <id>               terminate_confirmed: until RunPod no longer lists it; exit 1 if it still does"""
import json, os, sys, urllib.request
sys.path.insert(0, os.environ.get("HZ_COORD_DIR", os.path.expanduser("~/dev/hz-relcheck/coordinator")))
import sponsor_bot  # noqa: E402
GPU = "NVIDIA RTX PRO 6000 Blackwell Server Edition"
def main(a):
    api = sponsor_bot.RunPod.from_env()
    c = a[1] if len(a) > 1 else ""
    if c == "balance":
        r = urllib.request.Request("https://api.runpod.io/graphql", method="POST",
            data=json.dumps({"query": "query { myself { clientBalance } }"}).encode(),
            headers={"Content-Type": "application/json", "User-Agent": "hazync-libzync/1", "Authorization": f"Bearer {api.key}"})
        print(json.load(urllib.request.urlopen(r, timeout=60))["data"]["myself"]["clientBalance"]); return 0
    if c == "list":
        for p in api.pods(): print(p["id"], p["name"], p["status"], p["cost_per_hr"])
        print(f"({len(api.pods())} pods)"); return 0
    if c == "deploy" and len(a) == 6:  # deploy <name> <pubkey> <gpu id> <SECURE|COMMUNITY>
        assert a[2].startswith("hz-measure-") and a[5] in ("SECURE", "COMMUNITY")
        body = {"name": a[2], "cloud": a[5], "gpu": {"id": a[4], "count": 1}, "image": sponsor_bot.IMAGE,
                "disk": 150, "ports": ["22/tcp"], "env": {"PUBLIC_KEY": a[3]}}
        p = api._req("POST", "/pods", body)
        if not (p or {}).get("id"): print(f"no pod in answer: {str(p)[:200]}", file=sys.stderr); return 1
        print(p["id"], p.get("cost")); return 0
    if c == "ssh" and len(a) == 3:
        s = api.ssh_of(a[2]); s and print(s[0], s[1]); return 0
    if c == "terminate" and len(a) == 3:
        ok = sponsor_bot.terminate_confirmed(api, a[2]); print("TERMINATED" if ok else "STILL_LISTED"); return 0 if ok else 1
    print(__doc__, file=sys.stderr); return 2
if __name__ == "__main__":
    try: sys.exit(main(sys.argv))
    except sponsor_bot.RunPodError as e: print(f"RunPodError: {e}", file=sys.stderr); sys.exit(1)
