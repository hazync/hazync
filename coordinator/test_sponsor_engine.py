#!/usr/bin/env python3
"""The sponsor bot's engine pods (HAZYNC_SPONSOR_ENGINE=1, engine/BOARD_ENGINE.md): what a pod downloads, how it boots,
and how its work runs.

  1. release_files(): the CPU product host, hzengine (CUDA), the core image's key tarball, the worker and run-workers;
     the RISC Zero board's list is unchanged;
  2. the engine boot script parses, and RUN on a simulated pod (fake curl serving a local "release", fake nvidia-smi,
     apt-get and CUDA runtime, a fake host that proves only with the engine's environment) it prints SHA_OK, KEYS_OK,
     CUDART_OK, GPU_OK and the engine image id, and writes engine.env with the binary, the keys and the runtime path;
     a release whose host does not match want.txt prints SHA_BAD, and a pod with no CUDA 12.9 runtime CUDART_BAD;
  3. the work list a pod runs sources engine.env and runs the worker with the CPU host as HAZYNC_HOST;
  4. review R3-6 (28.11.2026): a file that does not match want.txt is NEVER executed and the boot stops non-zero (#1); a
     stale bad file on the pod is fetched again; a key file that is not the controller's pin is refused (KEYS_BAD); the
     smoke's verify-any runs under HAZYNC_ENGINE_VK + _PIN (the fake host refuses without both, #4); the controller
     accepts markers only as whole lines (#1); the pin comes from the key tarball checked against the verified
     manifest, and an operator pin that disagrees stops prepare (#4).

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


def simulate(tamper_host=False, no_runtime=False, stale_bad=False, wrong_pin=False):
    """Run the engine boot script on a fake pod. Returns (stdout, the pod dir, runtime dir, exit code, trace)."""
    T = tempfile.mkdtemp(prefix="sp_eng_")
    pod, rel, fbin, cuda = (os.path.join(T, d) for d in ("pod", "release", "bin", "cuda"))
    for d in (pod, rel, fbin, cuda):
        os.makedirs(d)
    cl = os.path.join(cuda, "targets", "x86_64-linux", "lib")
    # the "release": a host that proves only with the engine's environment (hzengine, keys, the runtime on the path)
    trace = os.path.join(T, "host_ran")
    write_exec(os.path.join(rel, "hazync-host-x86_64-linux-gnu"), f"""#!/bin/bash
echo "$*" >> {trace}
need() {{ [ -x "$HAZYNC_ENGINE" ] && [ -s "$HAZYNC_ENGINE_KEYS/agg.vk" ] && [ -e "{cl}/libcudart.so.12" ] && case ":$LD_LIBRARY_PATH:" in *":{cl}:"*) true ;; *) false ;; esac; }}
case "$1" in
  method-id) need && echo "METHOD_ID {ENG_ID}" || echo "METHOD_ID {'00' * 32}" ;;
  prove-range-bridge) need || {{ echo "no engine environment" >&2; exit 1; }}; echo proof > range_1.bin ;;
  verify-any) need && [ -n "$HAZYNC_ENGINE_VK" ] && [ -n "$HAZYNC_ENGINE_VK_PIN" ] \
      && [ "$(sha256sum "$HAZYNC_ENGINE_VK" | cut -c1-64)" = "$HAZYNC_ENGINE_VK_PIN" ] && [ -s "$2" ] \
      && echo "RANGE-OK lo=1 hi=1 tip=00" || {{ echo "REJECT no pinned key" >&2; exit 1; }} ;;
esac
""")
    write_exec(os.path.join(rel, "hazync-engine-x86_64-linux-gnu-cuda"), "#!/bin/bash\necho hzengine\n")
    write_exec(os.path.join(rel, "hazync-worker"), "#!/bin/bash\n")
    write_exec(os.path.join(rel, "hazync-run-workers.sh"), "#!/bin/bash\n")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for n in ("sdk.pk", "agg.vk", "openvm.toml", "guest.vmexe", "baseline.json", "image.json", "vk.bin"):
            data = f"{n} key bytes\n".encode()
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
    if stale_bad:   # a truncated host left on the pod by an earlier attempt: the old boot kept any non-empty file
        open(os.path.join(pod, "hazync-host-x86_64-linux-gnu"), "w").write("#!/bin/bash\necho stale\n")
    pin = hashlib.sha256(b"vk.bin key bytes\n").hexdigest()
    if wrong_pin:
        pin = hashlib.sha256(b"another key\n").hexdigest()
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
    script = sponsor_bot.boot_script("v9.9.9", vk_pin=pin).replace("W=/workspace", f"W={pod}") \
        .replace("/usr/local/cuda-12.9", cuda)
    sp = os.path.join(T, "boot.sh")
    open(sp, "w").write(script)
    r = subprocess.run(["bash", sp], capture_output=True, text=True, timeout=120,
                       env=dict(os.environ, PATH=f"{fbin}:{os.environ['PATH']}"))
    ran = open(trace).read() if os.path.exists(trace) else ""
    return r.stdout + r.stderr, pod, cl, r.returncode, ran


PIN = hashlib.sha256(b"vk.bin key bytes\n").hexdigest()
# 2. the boot script
boot = sponsor_bot.boot_script("v9.9.9", vk_pin=PIN)
check(subprocess.run(["bash", "-n"], input=boot, text=True).returncode == 0, "the engine boot script parses")
out, pod, cl, rc, _ = simulate()
for marker in ("SHA_OK", "KEYS_OK", "CUDART_OK", "GPU_OK", f"MID={ENG_ID}"):
    check(marker in out.splitlines(), f"a simulated engine pod prints {marker[:20]} (a whole line)")
check(sponsor_bot.boot_accepted(out, ENG_ID), "the controller accepts that boot")
env = open(os.path.join(pod, "engine.env")).read() if os.path.exists(os.path.join(pod, "engine.env")) else ""
check(f"HAZYNC_ENGINE={pod}/hazync-engine-x86_64-linux-gnu-cuda" in env and f"HAZYNC_ENGINE_KEYS={pod}/keys-core" in env
      and f"LD_LIBRARY_PATH={cl}" in env, "engine.env names hzengine, the unpacked keys and the CUDA 12.9 runtime")
check(f"HAZYNC_ENGINE_VK={pod}/keys-core/vk.bin" in env and f"HAZYNC_ENGINE_VK_PIN={PIN}" in env,
      "⛔ engine.env carries the pinned key (HAZYNC_ENGINE_VK + _PIN) for every verify-any on the pod (R3-6 #4)")
out_bad, _, _, rc_bad, ran_bad = simulate(tamper_host=True)
check("SHA_BAD hazync-host-x86_64-linux-gnu" in out_bad.splitlines() and rc_bad != 0,
      "⛔ a host that differs from the signed manifest stops the boot, non-zero (R3-6 #1)")
check(ran_bad == "" and "MID=" not in out_bad and "GPU_OK" not in out_bad,
      "⛔ and the tampered host is NEVER executed (no smoke, no method-id)")
check(not sponsor_bot.boot_accepted(out_bad, ENG_ID), "and the controller refuses that boot")
out_st, _, _, rc_st, _ = simulate(stale_bad=True)
check("SHA_OK" in out_st.splitlines() and rc_st == 0 and sponsor_bot.boot_accepted(out_st, ENG_ID),
      "a stale bad file on the pod is fetched again, then verified (the old boot kept any non-empty file)")
out_pin, _, _, rc_pin, ran_pin = simulate(wrong_pin=True)
check("KEYS_BAD keys-core/vk.bin is not the pinned key" in out_pin and rc_pin != 0 and "GPU_OK" not in out_pin,
      "⛔ a key file that is not the controller's pin stops the boot (KEYS_BAD) before the smoke")
check(not sponsor_bot.boot_accepted(
          "x SHA_OK KEYS_OK CUDART_OK GPU_OK\n" + f"echo MID={ENG_ID}\n", ENG_ID),
      "⛔ markers inside other text are not accepted: whole lines only (R3-6 #1)")
check(not sponsor_bot.boot_accepted("SHA_OK\nKEYS_OK\nCUDART_OK\nGPU_OK\nMID=" + "00" * 32 + "\n", ENG_ID),
      "another image id is refused")
try:
    sponsor_bot.boot_script("v9.9.9", engine=True)
    check(False, "an engine boot without the pin is refused")
except ValueError:
    check(True, "an engine boot without the pin is refused")
out_nort, _, _, _, _ = simulate(no_runtime=True)
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

# 4. the pin: the key tarball checked against the VERIFIED manifest, its single vk.bin hashed (R3-6 #4)
def pin_case(tamper=False, two_vk=False, op_pin=None):
    d = tempfile.mkdtemp(prefix="sp_pin_")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tf:
        for n in (["vk.bin", "vk.bin"] if two_vk else ["image.json", "vk.bin"]):
            data = f"{n} key bytes\n".encode()
            ti = tarfile.TarInfo(n)
            ti.size = len(data)
            tf.addfile(ti, io.BytesIO(data))
    blob = buf.getvalue()
    open(os.path.join(d, "SHA256SUMS.txt"), "w").write(f"{hashlib.sha256(blob).hexdigest()}  hazync-keys-core.tar.gz\n")
    served = blob + (b"x" if tamper else b"")
    r = sponsor_bot.SshRunner("/nonexistent", tempfile.mkdtemp(prefix="sp_home_"), "http://x/api/meta", workdir=d)
    r.dir = d
    real = sponsor_bot.urllib.request.urlretrieve
    sponsor_bot.urllib.request.urlretrieve = lambda url, path: open(path, "wb").write(served)
    old = os.environ.pop("SPONSOR_BOT_ENGINE_VK_PIN", None)
    if op_pin:
        os.environ["SPONSOR_BOT_ENGINE_VK_PIN"] = op_pin
    try:
        return r.engine_key_pin("v9.9.9")
    except SystemExit as e:
        return f"EXIT {e}"
    finally:
        sponsor_bot.urllib.request.urlretrieve = real
        os.environ.pop("SPONSOR_BOT_ENGINE_VK_PIN", None)
        if old is not None:
            os.environ["SPONSOR_BOT_ENGINE_VK_PIN"] = old


check(pin_case() == PIN, "prepare's pin is the SHA-256 of the verified key tarball's vk.bin")
check(str(pin_case(tamper=True)).startswith("EXIT") and "signed manifest" in str(pin_case(tamper=True)),
      "⛔ a key tarball that differs from the signed manifest stops prepare")
check(str(pin_case(two_vk=True)).startswith("EXIT"), "⛔ a tarball with two vk.bin members stops prepare")
check(pin_case(op_pin=PIN) == PIN and str(pin_case(op_pin="00" * 32)).startswith("EXIT"),
      "an operator pin (SPONSOR_BOT_ENGINE_VK_PIN) must agree with the release's key")

print(f"{'CONTROL: ' if CONTROL else ''}{fails} failure(s)")
# non-zero on any failure, the control included: CI's control step is `if ... --control; then exit 1` (it inverted
# before, so a control that failed as it must exited 0 and that CI step went red)
sys.exit(1 if fails else 0)
