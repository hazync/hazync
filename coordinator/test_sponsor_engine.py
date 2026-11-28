#!/usr/bin/env python3
"""The sponsor bot's engine pods (HAZYNC_SPONSOR_ENGINE=1, engine/BOARD_ENGINE.md): what a pod downloads, how it boots,
and how its work runs.

  1. release_files(): the CPU product host, hzengine (CUDA), the core image's key tarball, the worker and run-workers;
     the RISC Zero board's list is unchanged;
  2. the engine boot script parses, and RUN on a simulated pod (fake curl serving a local "release", fake nvidia-smi,
     apt-get and CUDA runtime, a fake host that proves only with the engine's environment) it prints SHA_OK, KEYS_OK,
     CUDART_OK, GPU_OK and the engine image id, and writes engine.env with the binary, the keys and the runtime path;
     a release whose host does not match want.txt prints SHA_BAD, and a pod with no CUDA 12.9 runtime CUDART_BAD;
  3. the work list a pod runs sources engine.env and runs the worker with the CPU host as HAZYNC_HOST.

  python3 test_sponsor_engine.py            # must PASS
  python3 test_sponsor_engine.py --control  # the engine pod gets the RISC Zero boot; MUST FAIL
"""
import hashlib
import io
import os
import stat
import subprocess
import sys
import tarfile
import tempfile

CONTROL = "--control" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.update(HAZYNC_LIBZYNC="1", HAZYNC_SPONSOR_ENGINE="1")
sys.path.insert(0, HERE)
import sponsor_bot  # noqa: E402

if CONTROL:
    sponsor_bot.ENGINE = False
    print("CONTROL: the engine pod gets the RISC Zero board's files and boot -- the checks below MUST fail")

fails = 0


def check(cond, what):
    global fails
    print(("  ok   " if cond else "  FAIL ") + what)
    fails += 0 if cond else 1


ENG_ID = "e3" * 32
FILES = sponsor_bot.release_files()

# 1. the files
check(FILES == ("hazync-worker", "hazync-run-workers.sh", "hazync-host-x86_64-linux-gnu",
                "hazync-engine-x86_64-linux-gnu-cuda", "hazync-keys-core.tar.gz"),
      f"an engine pod downloads the CPU host, hzengine (CUDA) and the core key tarball ({FILES})")
check(sponsor_bot.release_files(False) == ("hazync-worker", "hazync-run-workers.sh", "hazync-host-x86_64-linux-gnu-cuda"),
      "the RISC Zero board's pods download what they did")


def write_exec(path, body):
    with open(path, "w") as f:
        f.write(body)
    os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)


def simulate(tamper_host=False, no_runtime=False):
    """Run the engine boot script on a fake pod. Returns (stdout, the pod dir)."""
    T = tempfile.mkdtemp(prefix="sp_eng_")
    pod, rel, fbin, cuda = (os.path.join(T, d) for d in ("pod", "release", "bin", "cuda"))
    for d in (pod, rel, fbin, cuda):
        os.makedirs(d)
    cl = os.path.join(cuda, "targets", "x86_64-linux", "lib")
    # the "release": a host that proves only with the engine's environment (hzengine, keys, the runtime on the path)
    write_exec(os.path.join(rel, "hazync-host-x86_64-linux-gnu"), f"""#!/bin/bash
need() {{ [ -x "$HAZYNC_ENGINE" ] && [ -s "$HAZYNC_ENGINE_KEYS/agg.vk" ] && [ -e "{cl}/libcudart.so.12" ] && case ":$LD_LIBRARY_PATH:" in *":{cl}:"*) true ;; *) false ;; esac; }}
case "$1" in
  method-id) need && echo "METHOD_ID {ENG_ID}" || echo "METHOD_ID {'00' * 32}" ;;
  prove-range-bridge) need || {{ echo "no engine environment" >&2; exit 1; }}; echo proof > range_1.bin ;;
  verify-any) need && [ -s "$2" ] && echo "RANGE-OK lo=1 hi=1 tip=00" ;;
esac
""")
    write_exec(os.path.join(rel, "hazync-engine-x86_64-linux-gnu-cuda"), "#!/bin/bash\necho hzengine\n")
    write_exec(os.path.join(rel, "hazync-worker"), "#!/bin/bash\n")
    write_exec(os.path.join(rel, "hazync-run-workers.sh"), "#!/bin/bash\n")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for n in ("sdk.pk", "agg.vk", "openvm.toml", "guest.vmexe", "baseline.json", "image.json", "vk.bin"):
            data = f"{n}\n".encode()
            ti = tarfile.TarInfo(n)
            ti.size = len(data)
            tf.addfile(ti, io.BytesIO(data))
    open(os.path.join(rel, "hazync-keys-core.tar.gz"), "wb").write(buf.getvalue())
    with open(os.path.join(pod, "want.txt"), "w") as f:
        for n in FILES:
            if not os.path.exists(os.path.join(rel, n)):      # (--control: the RISC Zero list names a file not here)
                continue
            f.write(f"{hashlib.sha256(open(os.path.join(rel, n), 'rb').read()).hexdigest()}  {n}\n")
    if tamper_host:
        with open(os.path.join(rel, "hazync-host-x86_64-linux-gnu"), "a") as f:
            f.write("# changed after signing\n")
    open(os.path.join(pod, "leaf_1.bin"), "wb").write(b"LZLEAF02")
    # fake tools: curl copies from the local release; nvidia-smi names a card; apt-get "installs" the runtime
    write_exec(os.path.join(fbin, "curl"), f"""#!/bin/bash
out=""; url=""
while [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift 2 ;; -*) shift ;; *) url=$1; shift ;; esac; done
[ -f "{rel}/$(basename "$url")" ] || exit 22
cp "{rel}/$(basename "$url")" "$out"
""")
    write_exec(os.path.join(fbin, "nvidia-smi"), "#!/bin/bash\necho 'NVIDIA RTX A6000'\n")
    write_exec(os.path.join(fbin, "apt-get"),
               "#!/bin/bash\n" + ("exit 100\n" if no_runtime else
                                  f'case "$*" in *cuda-cudart-12-9*) mkdir -p {cl} && touch {cl}/libcudart.so.12 ;; esac\n'))
    write_exec(os.path.join(fbin, "ldconfig"), "#!/bin/bash\n")
    write_exec(os.path.join(fbin, "dpkg"), "#!/bin/bash\n" + ("exit 1\n" if no_runtime else ""))
    script = sponsor_bot.boot_script("v9.9.9").replace("W=/workspace", f"W={pod}") \
        .replace("/usr/local/cuda-12.9", cuda)
    sp = os.path.join(T, "boot.sh")
    open(sp, "w").write(script)
    r = subprocess.run(["bash", sp], capture_output=True, text=True, timeout=120,
                       env=dict(os.environ, PATH=f"{fbin}:{os.environ['PATH']}"))
    return r.stdout + r.stderr, pod, cl


# 2. the boot script
boot = sponsor_bot.boot_script("v9.9.9")
check(subprocess.run(["bash", "-n"], input=boot, text=True).returncode == 0, "the engine boot script parses")
out, pod, cl = simulate()
for marker in ("SHA_OK", "KEYS_OK", "CUDART_OK", "GPU_OK", f"MID={ENG_ID}"):
    check(marker in out, f"a simulated engine pod prints {marker[:20]}")
env = open(os.path.join(pod, "engine.env")).read() if os.path.exists(os.path.join(pod, "engine.env")) else ""
check(f"HAZYNC_ENGINE={pod}/hazync-engine-x86_64-linux-gnu-cuda" in env and f"HAZYNC_ENGINE_KEYS={pod}/keys-core" in env
      and f"LD_LIBRARY_PATH={cl}" in env, "engine.env names hzengine, the unpacked keys and the CUDA 12.9 runtime")
out_bad, _, _ = simulate(tamper_host=True)
check("SHA_BAD" in out_bad, "a host that differs from the signed manifest prints SHA_BAD")
out_nort, _, _ = simulate(no_runtime=True)
check("CUDART_BAD" in out_nort and "GPU_OK" not in out_nort,
      "a pod with no CUDA 12.9 runtime prints CUDART_BAD and does not pass the smoke")

# 3. the work list
class FakePod:
    id, name = "p1", "hz-sponsor-p1"


runner = sponsor_bot.SshRunner("/nonexistent", tempfile.mkdtemp(prefix="sp_home_"), "http://x/api/meta",
                               workdir=tempfile.mkdtemp(prefix="sp_work_"))
sent = {}
runner._ssh = lambda pod, cmd, timeout=60: subprocess.CompletedProcess([], 0, "", "")
def _scp(pod, paths, dest):
    for p in paths:
        if os.path.basename(p).startswith("run-"):
            sent["script"] = open(p).read()
    return subprocess.CompletedProcess([], 0, "", "")
runner._scp = _scp
pod = FakePod()
pod.identities = {"t1"}
runner.start(pod, [{"height": 5, "home": "/h", "pubkey": "pk", "handle": "h", "tag": "t1"}])
s = sent.get("script", "")
check(". /workspace/engine.env" in s and "HAZYNC_HOST=/workspace/hazync-host " in s,
      "the work list sources engine.env and runs the worker with the CPU host")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
if CONTROL:
    sys.exit(0 if fails else 1)
sys.exit(1 if fails else 0)
