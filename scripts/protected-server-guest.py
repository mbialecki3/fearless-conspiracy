#!/usr/bin/env python3
"""Destructive resource tests: only run in the marked, disposable QEMU VM."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import traceback

OUT = Path("/results")
LAUNCHER = "/lab/scripts/protected-server.py"
CGROUP = Path("/sys/fs/cgroup/user.slice")


def wait_for(condition, label, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if condition():
            return
        time.sleep(0.05)
    raise RuntimeError("timeout: " + label)


def values(path):
    return dict(line.split() for line in path.read_text().splitlines())


def main():
    if not Path("/FC_DISPOSABLE_VM").exists() or Path("/sys/class/dmi/id/product_name").read_text().strip() != "FC proc visibility disposable VM":
        raise SystemExit("Refusing: resource tests require the dedicated disposable VM")
    if Path("/proc/1/comm").read_text().strip() != "systemd":
        raise SystemExit("The test VM must boot real systemd")
    OUT.mkdir(exist_ok=True, mode=0o700)
    OUT.chmod(0o700)
    os.umask(0o077)
    evidence = {"kernel": os.uname().release, "measurements": [], "pass": False}
    handles, processes, units = [], [], []
    server = None

    def check(label, condition, details=None):
        evidence["measurements"].append({"label": label, "pass": bool(condition), "details": details})
        assert condition, (label, details)

    def run_as(uid, code, *args, timeout=15):
        return subprocess.run(["/usr/bin/setpriv", "--reuid", str(uid), "--regid", str(uid),
                               "--clear-groups", "--no-new-privs", "/usr/bin/python3", "-I", "-c", code, *map(str, args)],
                              capture_output=True, text=True, timeout=timeout)

    def launch(command, label, uid=None):
        handle = (OUT / f"{label}.log").open("w")
        handles.append(handle)
        if uid is not None:
            unit = f"fc-test-{label}"
            units.append(unit + ".scope")
            command = ["/usr/bin/systemd-run", "--quiet", "--scope", f"--unit={unit}", f"--slice=user-{uid}.slice",
                       "/usr/bin/setpriv", "--reuid", str(uid), "--regid", str(uid), "--clear-groups", "--no-new-privs", *command]
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=handle, stderr=subprocess.STDOUT, text=True, cwd="/")
        processes.append(process)
        return process

    def log(label):
        return (OUT / f"{label}.log").read_text()

    def send(process, command):
        process.stdin.write(command + "\n")
        process.stdin.flush()

    try:
        policy = subprocess.run([sys.executable, "/lab/scripts/test-protected-server.py"], capture_output=True, text=True)
        check("launcher policy tests", policy.returncode == 0, policy.stdout + policy.stderr)
        before_mount = Path("/proc/mounts").read_text()
        denied = subprocess.run([sys.executable, LAUNCHER, "--player", "fearless_p1", "--player", "fearless_p1", "--player", "fearless_p3"], capture_output=True, text=True)
        check("duplicate accounts rejected before remount", denied.returncode != 0 and "distinct Linux UIDs" in denied.stderr and Path("/proc/mounts").read_text() == before_mount)
        sudoers = Path("/etc/sudoers")
        policy = sudoers.read_text()
        audit = subprocess.run(["/usr/bin/sudo", "-n", "-l", "-U", "fearless_p1"], capture_output=True, text=True)
        check("sudo policy can be checked", audit.returncode in (0, 1) and "not allowed to run sudo" in audit.stdout and not audit.stderr, {"exit": audit.returncode, "stdout": audit.stdout, "stderr": audit.stderr})
        try:
            sudoers.write_text(policy + "\nfearless_p1 ALL=(root) NOPASSWD: /usr/bin/true\n")
            denied = subprocess.run([sys.executable, LAUNCHER, "--player", "fearless_p1", "--player", "fearless_p2", "--player", "fearless_p3"], capture_output=True, text=True)
            check("custom sudo privileges rejected before remount", denied.returncode != 0 and "must have no sudo permissions" in denied.stderr and Path("/proc/mounts").read_text() == before_mount, denied.stderr)
        finally:
            sudoers.write_text(policy)
        with subprocess.Popen(["/usr/bin/sleep", "infinity"], user=2001, group=2001, extra_groups=[]) as unmanaged:
            try:
                denied = subprocess.run([sys.executable, LAUNCHER, "--player", "fearless_p1", "--player", "fearless_p2", "--player", "fearless_p3"], capture_output=True, text=True)
                check("existing unlimited player processes rejected before remount", denied.returncode != 0 and "outside its systemd user slice" in denied.stderr and Path("/proc/mounts").read_text() == before_mount, denied.stderr)
            finally:
                unmanaged.terminate()
                unmanaged.wait()
        args = [sys.executable, LAUNCHER]
        for number in range(1, 10):
            args += ["--player", f"fearless_p{number}"]
        args += ["--", "--manual", "--scenario", "--preset", "full9"]
        server = launch(args, "server")
        for _ in range(4):
            send(server, "")
        socket = Path("/run/fearless-conspiracy/game.sock")
        wait_for(lambda: socket.exists() or server.poll() is not None, "protected launch")
        check("protected launcher starts server", socket.exists() and server.poll() is None, log("server"))
        pids = []
        for directory in Path("/proc").iterdir():
            if directory.name.isdigit():
                try:
                    text = (directory / "status").read_text()
                    if re.search(r"^Uid:\s+2000\s", text, re.M) and "fearless-conspiracy-server" in (directory / "cmdline").read_text():
                        pids.append(int(directory.name))
                except FileNotFoundError:
                    pass
        check("one unprivileged game server", len(pids) == 1, pids)
        pid = pids[0]
        status = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines() if ":" in line)
        check("game has no root credentials or capabilities", set(status["Uid"].split()) == {"2000"} and all(int(status[key], 16) == 0 for key in ("CapEff", "CapPrm", "CapInh", "CapAmb", "CapBnd")) and status["NoNewPrivs"].strip() == "1", status)
        mounts = [line for line in Path("/proc/mounts").read_text().splitlines() if line.split()[1] == "/proc"]
        check("global proc restriction active", any("hidepid=invisible" in line or "hidepid=2" in line for line in mounts), mounts)
        limits = {}
        for uid in range(2000, 2010):
            root = CGROUP / f"user-{uid}.slice"
            limits[uid] = {name: (root / name).read_text().strip() for name in ("cpu.max", "memory.high", "memory.max", "memory.swap.max", "pids.max")}
            check(f"kernel limits for UID {uid}", limits[uid]["memory.max"] == str((512 if uid == 2000 else 256) * 1024 * 1024)
                  and limits[uid]["memory.swap.max"] == "0" and limits[uid]["pids.max"] == ("128" if uid == 2000 else "64")
                  and limits[uid]["cpu.max"].split()[0] == ("100000" if uid == 2000 else "25000"), limits[uid])
        evidence["kernel_limits"] = limits
        for uid in range(2001, 2010):
            result = run_as(uid, 'from pathlib import Path; import sys; Path(f"/proc/{sys.argv[1]}/task").iterdir().__next__()', pid)
            other = CGROUP / f"user-{2002 if uid == 2001 else 2001}.slice/cpu.stat"
            cgroup_read = run_as(uid, 'from pathlib import Path; import sys; print(Path(sys.argv[1]).read_text())', other)
            write_limit = run_as(uid, 'from pathlib import Path; import sys; Path(sys.argv[1]).write_text("max")', CGROUP / f"user-{uid}.slice/memory.max")
            check(f"UID {uid} cannot inspect or remove limits", result.returncode != 0 and cgroup_read.returncode != 0 and write_limit.returncode != 0)
        outsider = run_as(2010, 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print(s.recv(4096))', socket)
        check("unregistered account rejected", "not registered" in outsider.stdout, outsider.stdout + outsider.stderr)
        outside = run_as(2001, 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print(s.recv(4096))', socket)
        check("registered account outside its limited slice rejected", "resource-limited login session" in outside.stdout, outside.stdout + outside.stderr)
        result = run_as(2000, 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print(s.recv(4096))', socket)
        check("server account cannot join", "not registered" in result.stdout, result.stdout + result.stderr)
        clients = {}
        for number in range(1, 10):
            clients[number] = launch(["/lab/bin/fearless-conspiracy-client", str(socket), f"Player{number}", "--plain"], f"player{number}", 2000+number)
            wait_for(lambda n=number: f"#{n} Player{n} joined" in log("server"), f"player {number} admission")
        for process in clients.values():
            send(process, "/ready")
            time.sleep(0.1)
        wait_for(lambda: all("You are the " in log(f"player{number}") for number in clients), "role reveal")
        for uid in range(2001, 2010):
            other = CGROUP / f"user-{2002 if uid == 2001 else 2001}.slice/cpu.stat"
            result = run_as(uid, 'from pathlib import Path; import sys; print(Path(sys.argv[1]).read_text())', other)
            check(f"UID {uid} remains isolated after new sessions start", result.returncode != 0)
        roles = {number: re.search(r"You are the (.+?) \((Town|Mafia|Neutral)\)", log(f"player{number}")).groups() for number in clients}
        mafia = [number for number in roles if roles[number][1] == "Mafia"]
        town = next(number for number in roles if roles[number][0] == "Lookout")
        send(server, "advance")
        wait_for(lambda: "Day 1 Introductions" in log("server"), "day")
        send(server, "advance")
        wait_for(lambda: "Night 1 has begun" in log(f"player{town}"), "night")
        send(clients[mafia[0]], "/mafia protected-night-probe")
        wait_for(lambda: all("protected-night-probe" in log(f"player{number}") for number in mafia), "private mafia delivery")
        check("private night delivery works without leaking to Town", "protected-night-probe" not in log(f"player{town}"))
        root = CGROUP / "user-2001.slice"
        start = values(root / "cpu.stat")
        cpu = launch([sys.executable, "-I", "-c", 'import time; end=time.monotonic()+3;\nwhile time.monotonic()<end: pass'], "cpu-stress", 2001)
        cpu.wait(timeout=20)
        end = values(root / "cpu.stat")
        check("CPU load is throttled", int(end["nr_throttled"]) > int(start["nr_throttled"]) and int(end["usage_usec"]) - int(start["usage_usec"]) < 1_800_000, {"before": start, "after": end})
        start = values(root / "pids.events")
        fork_code = '''import os, time, signal, errno, json
children=[]
try:
    for _ in range(100):
        try: pid=os.fork()
        except OSError as exc:
            assert exc.errno == errno.EAGAIN
            print(json.dumps({"fork_blocked":True,"children":len(children)}), flush=True)
            break
        if pid==0:
            time.sleep(5)
            os._exit(0)
        children.append(pid)
finally:
    for pid in children: os.kill(pid, signal.SIGKILL)
    for pid in children: os.waitpid(pid, 0)
'''
        forks = launch([sys.executable, "-I", "-c", fork_code], "fork-stress", 2001)
        forks.wait(timeout=20)
        end = values(root / "pids.events")
        check("process creation stops at the task limit", forks.returncode == 0 and '"fork_blocked": true' in log("fork-stress") and int(end["max"]) > int(start["max"]), log("fork-stress"))
        start = values(root / "memory.events")
        memory = launch([sys.executable, "-I", "-c", 'from pathlib import Path; Path("/proc/self/oom_score_adj").write_text("1000"); data=bytearray(320*1024*1024)'], "memory-stress", 2001)
        wait_for(lambda: int(values(root / "memory.events")["high"]) > int(start["high"]) or memory.poll() is not None, "memory throttling")
        check("memory pressure activates the soft throttle", int(values(root / "memory.events")["high"]) > int(start["high"]))
        # Exercise the hard cap separately: soft throttling can intentionally
        # stall allocation indefinitely before the hard limit is reached.
        high = (root / "memory.high").read_text()
        try:
            (root / "memory.high").write_text("max")
            memory.wait(timeout=60)
        finally:
            (root / "memory.high").write_text(high)
        end = values(root / "memory.events")
        check("memory exhaustion is contained to the player slice", memory.returncode != 0 and int(end["oom_kill"]) > int(start["oom_kill"]), {"before": start, "after": end, "exit": memory.returncode})
        check("game survives the resource tests", server.poll() is None and all(process.poll() is None for process in clients.values()))
        send(clients[town], "/players")
        time.sleep(0.5)
        send(server, "shutdown")
        server.wait(timeout=20)
        check("game shuts down normally", server.returncode == 0, log("server")[-1000:])
        check("protections remain after shutdown", "hidepid=invisible" in Path("/proc/mounts").read_text() and (root / "pids.max").read_text().strip() == "64")
        evidence["pass"] = True
    except BaseException:
        evidence["error"] = traceback.format_exc()
        traceback.print_exc()
    finally:
        for unit in units:
            subprocess.run(["/usr/bin/systemctl", "stop", unit], capture_output=True)
        for process in reversed(processes):
            if process.poll() is None:
                process.kill()
            process.wait()
        for handle in handles:
            handle.close()
        (OUT / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return 0 if evidence["pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
