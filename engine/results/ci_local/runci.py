import yaml, subprocess, os, sys, time, json, re
TREE, S = sys.argv[1], sys.argv[2]
jobs = sys.argv[3].split(",")
d = yaml.safe_load(open(f"{TREE}/.github/workflows/adversarial.yml"))
res = []
for j in jobs:
    v = d["jobs"][j]
    subprocess.run("git clean -fdxq -e target -e node_modules", shell=True, cwd=TREE)
    rt = f"{S}/rt_{j}"; os.makedirs(rt, exist_ok=True)
    out, summ, envf = f"{rt}/output", f"{rt}/summary", f"{rt}/env"
    for f in (out, summ, envf): open(f, "w").close()
    jobfail = False
    for i, s in enumerate(v.get("steps", [])):
        name = s.get("name") or s.get("uses")
        if "run" not in s:
            res.append(dict(job=j, i=i, name=name, status="action", note=s.get("uses"))); continue
        cond = s.get("if")
        # inventory: every step runs even after a failure (CI would stop the job); after_fail marks those
        env = dict(os.environ)
        env.update(PATH=f"{S}/shim:" + env["PATH"], CI="true", GITHUB_ACTIONS="true", GITHUB_OUTPUT=out,
                   GITHUB_STEP_SUMMARY=summ, GITHUB_ENV=envf, RUNNER_TEMP=rt, GITHUB_WORKSPACE=TREE,
                   GITHUB_EVENT_NAME="push", GITHUB_REF="refs/heads/main", CARGO_BUILD_JOBS="4")
        for k, val in (s.get("env") or {}).items():
            val = str(val)
            val = val.replace("${{ github.event_name }}", "push")
            env[k] = val
        # GITHUB_ENV lines from earlier steps
        for line in open(envf):
            if "=" in line and "<<" not in line:
                k, _, val = line.rstrip("\n").partition("="); env[k] = val
        wd = os.path.join(TREE, s.get("working-directory", ""))
        log = f"{S}/logs/{j}_{i:03d}.log"
        t0 = time.time()
        try:
            p = subprocess.run(["bash", "--noprofile", "--norc", "-eo", "pipefail", "-c", s["run"]], cwd=wd, env=env,
                               stdout=open(log, "w"), stderr=subprocess.STDOUT, timeout=int(sys.argv[4]) if len(sys.argv) > 4 else 3600)
            rc = p.returncode
        except subprocess.TimeoutExpired:
            rc = "timeout"
        dt = round(time.time() - t0)
        st = "pass" if rc == 0 else "FAIL"
        res.append(dict(job=j, i=i, name=name, status=st, rc=rc, secs=dt, log=log, after_fail=jobfail and rc == 0))
        if rc != 0: jobfail = True
        print(f"{j}[{i}] {st} {dt}s {name[:90]}", flush=True)
    json.dump(res, open(f"{S}/res_{'_'.join(jobs)}.json", "w"), indent=1)
