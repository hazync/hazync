"""timed.py <log> -- <command...>: runs the command with stdout+stderr to <log>, then appends one line
    TIMED wall_s=<s> user_s=<s> sys_s=<s> maxrss_kb=<kB> rc=<exit> load1=<1-min load at the start> load1_end=<at the end>
(the box is shared: the load average is recorded so a wall time can be read against it)."""
import os, resource, subprocess, sys, time
log = sys.argv[1]; cmd = sys.argv[sys.argv.index("--") + 1:]
l0 = os.getloadavg()[0]; t0 = time.time()
with open(log, "ab") as f:
    rc = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT).returncode
dt = time.time() - t0; ru = resource.getrusage(resource.RUSAGE_CHILDREN)
with open(log, "a") as f:
    f.write(f"TIMED wall_s={dt:.1f} user_s={ru.ru_utime:.1f} sys_s={ru.ru_stime:.1f} maxrss_kb={ru.ru_maxrss} rc={rc} "
            f"load1={l0:.1f} load1_end={os.getloadavg()[0]:.1f}\n")
sys.exit(rc)
