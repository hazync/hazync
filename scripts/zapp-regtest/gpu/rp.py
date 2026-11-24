#!/usr/bin/env python3
"""RunPod REST v2 calls for gpu_run.sh, through the board's own client (coordinator/sponsor_bot.py RunPod):
   rp.py deploy <name> <gpu type id> <ssh public key> [gpus]   prints the pod id (SECURE cloud, the bot's image)
   rp.py ssh <pod id>                                   prints "<ip> <port>" of the public SSH, or nothing yet
   rp.py terminate <pod id>                             DELETE; a pod RunPod no longer has is already gone
The key is read from $RP (gpu_run.sh reads ~/.hazync/runpod.key). Exit 1 with the reason on stderr on failure."""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "coordinator"))
import sponsor_bot  # noqa: E402

def main(argv):
    api = sponsor_bot.RunPod(os.environ["RP"])
    cmd = argv[1] if len(argv) > 1 else ""
    try:
        if cmd == "deploy" and len(argv) in (5, 6):
            n = int(argv[5]) if len(argv) == 6 else 1
            p = api.create_pod(argv[2], argv[3], argv[4], ports=("22/tcp",), disk_gb=80, gpu_count=n)
            if not (p or {}).get("id"):
                print(f"deploy: no pod in the answer: {p}", file=sys.stderr); return 1
            print(p["id"]); return 0
        if cmd == "ssh" and len(argv) == 3:
            s = api.ssh_of(argv[2])
            if s:
                print(s[0], s[1])
            return 0
        if cmd == "terminate" and len(argv) == 3:
            api.terminate(argv[2]); return 0
    except sponsor_bot.RunPodError as e:
        print(f"{cmd}: {e}", file=sys.stderr); return 1
    print(__doc__, file=sys.stderr); return 2

if __name__ == "__main__":
    sys.exit(main(sys.argv))
