#!/usr/bin/env python3
"""Destructive resource tests: only run in the marked, disposable QEMU VM."""
import json
import ctypes
import importlib.util
import os
from pathlib import Path
import re
import signal
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

    spec = importlib.util.spec_from_file_location("protected_launcher", LAUNCHER)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)

    def environment_state():
        return {"proc": helper.proc_mount(), "overrides": {
            str(path): {"link": os.readlink(path)} if path.is_symlink() else
            {"data": path.read_bytes().hex(), "mode": path.stat().st_mode & 0o777}
            for path in helper.OVERRIDES.glob("user-*.slice.d/*.conf")}}

    def restored(label, before):
        after = environment_state()
        check(label, after == before and not (helper.STATE / "isolation.json").exists(),
              {"before": before, "after": after})

    try:
        policy = subprocess.run([sys.executable, "/lab/scripts/test-protected-server.py"], capture_output=True, text=True)
        check("launcher policy tests", policy.returncode == 0, policy.stdout + policy.stderr)
        check("VM starts without game accounts", "fearless_" not in Path("/etc/passwd").read_text())
        bootstrap_before = environment_state()
        bootstrap = launch(["/bin/bash", "/lab/server", "--players", "9", "--", "--manual", "--scenario", "--preset", "full9"], "bootstrap")
        for _ in range(4):
            send(bootstrap, "")
        socket = Path("/run/fearless-conspiracy/game.sock")
        wait_for(lambda: socket.exists() or bootstrap.poll() is not None, "automatic account creation", timeout=90)
        check("automatic startup creates accounts and starts game", socket.exists() and bootstrap.poll() is None, log("bootstrap"))
        check("default server wrapper uses protected startup", "Verified player isolation:" in log("bootstrap"))
        state = Path("/var/lib/fearless-conspiracy")
        registry = json.loads((state / "accounts.json").read_text())
        check("ten dedicated accounts created", len(registry) == 10 and {record["uid"] for record in registry.values()} == set(range(2000, 2010)))
        check("created accounts have only their private groups", all(set(os.getgrouplist(name, record["gid"])) == {record["gid"]} for name, record in registry.items()))
        check("credentials and registry are root-only", state.stat().st_mode & 0o777 == 0o700 and all((state / name).stat().st_uid == 0 and (state / name).stat().st_mode & 0o777 == 0o600 for name in ("accounts.json", "player-logins.txt")))
        shadow = {line.split(":")[0]: line.split(":")[1] for line in Path("/etc/shadow").read_text().splitlines()}
        check("server login is locked", shadow["fearless_host"].startswith("!") and registry["fearless_host"]["password"] is None)
        crypt = ctypes.CDLL("libcrypt.so.1")
        crypt.crypt.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
        crypt.crypt.restype = ctypes.c_char_p
        for number in range(1, 10):
            name = f"fearless_p{number}"
            password = registry[name]["password"]
            home = Path(registry[name]["home"]).stat()
            check(f"{name} has a usable private login", crypt.crypt(password.encode(), shadow[name].encode()).decode() == shadow[name]
                  and home.st_mode & 0o777 == 0o700 and home.st_uid == registry[name]["uid"] and home.st_gid == registry[name]["gid"])
        check("players cannot read the login file", run_as(2001, 'from pathlib import Path; import sys; Path(sys.argv[1]).read_text()', state / "player-logins.txt").returncode != 0)
        check("new accounts can execute staged client", run_as(2001, 'import os,sys; assert os.access(sys.argv[1], os.X_OK)', "/run/fearless-conspiracy/fearless-conspiracy-client").returncode == 0)
        send(bootstrap, "shutdown")
        bootstrap.wait(timeout=20)
        check("bootstrap game shuts down normally", bootstrap.returncode == 0 and not socket.exists())
        def players_removed(count):
            names = {f"fearless_p{number}" for number in range(1, count+1)}
            users = {line.split(":")[0] for line in Path("/etc/passwd").read_text().splitlines()}
            groups = {line.split(":")[0] for line in Path("/etc/group").read_text().splitlines()}
            return not names & users and not names & groups and all(not Path(f"/home/{name}").exists() for name in names)

        check("normal shutdown removes player accounts homes and groups", players_removed(9))
        check("normal shutdown removes saved player credentials", set(json.loads((state / "accounts.json").read_text())) == {"fearless_host"} and not (state / "player-logins.txt").exists())
        restored("bootstrap restores proc and removes only game limit overrides", bootstrap_before)
        helper.provision_accounts("fearless_host", 9)  # Fixtures for legacy-account refusal checks.
        baseline_registry = (state / "accounts.json").read_bytes()
        baseline_shadow = Path("/etc/shadow").read_bytes()
        subprocess.run(["/usr/sbin/groupadd", "--gid", "1000", "outsider"], check=True, capture_output=True)
        subprocess.run(["/usr/sbin/useradd", "--uid", "1000", "--gid", "1000", "--no-create-home", "outsider"], check=True, capture_output=True)
        # Existing-account mode must continue refusing unrelated accounts before
        # changing restrictions. Automatic mode must not adopt them either.
        collision = subprocess.run([sys.executable, LAUNCHER, "--server-user", "outsider", "--players", "3"], capture_output=True, text=True)
        check("automatic provisioning refuses unrelated existing accounts", collision.returncode != 0 and "not managed" in collision.stderr)
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
        args = ["/bin/bash", "/lab/server", "--players", "9"]
        args += ["--", "--manual", "--scenario", "--preset", "full9"]
        game_before = environment_state()
        server = launch(args, "server")
        for _ in range(4):
            send(server, "")
        socket = Path("/run/fearless-conspiracy/game.sock")
        wait_for(lambda: socket.exists() or server.poll() is not None, "protected launch")
        check("protected launcher starts server", socket.exists() and server.poll() is None, log("server"))
        check("restarting preserves managed accounts and passwords", (state / "accounts.json").read_bytes() == baseline_registry
              and all(line in Path("/etc/shadow").read_bytes().splitlines() for line in baseline_shadow.splitlines()))
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
        outsider = run_as(1000, 'import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); print(s.recv(4096))', socket)
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
        # Capture a complete match on distinct accounts for the new report.
        # Each command waits for its actual reply rather than assuming delivery.
        def command(number, text, reply):
            offset = len(log(f"player{number}"))
            send(clients[number], text)
            wait_for(lambda: reply in log(f"player{number}")[offset:], text)

        def advance(marker):
            offsets = {n: len(log(f"player{n}")) for n in clients}
            send(server, "advance")
            wait_for(lambda: all(marker in log(f"player{n}")[offsets[n]:] for n in clients), marker)

        command(1, "/investigate 7", "Night choice set to #7")
        command(2, "/heal 1", "Night choice set to #1")
        command(3, "/watch 1", "Night choice set to #1")
        command(6, "/kill 1", "Night choice set to #1")
        command(8, "/will set I claimed Town, but I was the Framer.", "Will replaced.")
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
        advance("Nobody died during the night.")
        check("night actions still resolve after resource pressure", "#7 appears suspicious" in log("player1")
              and "Your protected target #1 was attacked" in log("player2")
              and "visitors to #1: #2, #7" in log("player3"))

        living = set(range(1, 10))

        def trial(accused, final=False):
            advance("Discussion has begun.")
            command(1, "/say My investigation found player 7 suspicious.", "My investigation found")
            advance("Nominations are open.")
            for voter in sorted(living - {accused})[:len(living)//2+1]:
                command(voter, f"/vote {accused}", f"nominated #{accused}")
            wait_for(lambda: f"#{accused} is on trial" in log("player1"), "trial begins")
            command(accused, "/say I claim to be Town.", "I claim to be Town.")
            advance("Judgment voting is open")
            for voter in sorted(living - {accused}):
                command(voter, "/guilty", "Judgment choice set to guilty.")
            advance("Verdict: Convicted.")
            command(accused, "/say These are my last words.", "These are my last words.")
            advance("Final faction result: Town" if final else "The public execution is complete.")
            living.remove(accused)
            check(f"trial and execution of player {accused}", f"#{accused} Player{accused} was publicly executed" in log("player1"))

        trial(8)
        command(6, "/kill 4", "Night choice set to #4")
        advance("#4 Player4 died.")
        living.remove(4)
        check("night death is broadcast", all("#4 Player4 died." in log(f"player{n}") for n in clients))
        trial(6)
        advance("Nobody died during the night.")
        trial(7, final=True)
        command(4, "/say Good game from the dead player.", "Good game from the dead player.")
        check("protected match reaches Town victory and postgame", all("Final faction result: Town" in log(f"player{n}")
              and "Good game from the dead player." in log(f"player{n}") for n in clients))
        leftover = launch(["/usr/bin/sleep", "infinity"], "leftover-session", 2001)
        sleeper_pids = root / "fc-test-leftover-session.scope/cgroup.procs"
        wait_for(lambda: sleeper_pids.exists() and any(re.search(r"^Uid:\s+2001\s", Path(f"/proc/{pid}/status").read_text(), re.M)
                 for pid in sleeper_pids.read_text().split()), "persistent player session")
        check("player session remains active before shutdown", leftover.poll() is None)
        send(server, "shutdown")
        server.wait(timeout=20)
        check("game shuts down normally", server.returncode == 0, log("server")[-1000:])
        wait_for(lambda: leftover.poll() is not None, "player session termination")
        check("cleanup terminates remaining player sessions", leftover.poll() is not None)
        check("live-player shutdown removes accounts homes and groups", players_removed(9))
        check("unrelated account survives cleanup", "outsider:" in Path("/etc/passwd").read_text())
        restored("live-player shutdown restores original proc and resource configuration", game_before)

        # Existing-account mode must leave both identities and home data alone.
        helper.provision_accounts("fearless_host", 3)
        run_as(2001, 'from pathlib import Path; Path("/home/fearless_p1/keep-me").write_text("player notes")').check_returncode()
        # Check preservation of nondefault proc options, administrator limits,
        # an unrelated override, an active slice, and cgroup permissions.
        helper.remount_proc(helper.proc_mount(), "hidepid=1,gid=2999", "cannot set test baseline")
        helper.command("systemctl", "set-property", "--runtime", "user-2001.slice", "CPUQuota=60%", "MemoryHigh=300M",
                       "MemoryMax=400M", "MemorySwapMax=64M", "TasksMax=90", "CPUWeight=42")
        helper.command("systemctl", "start", "user-2001.slice")
        original_slice = CGROUP / "user-2001.slice"
        # kernfs may support only the basic ACL represented by mode bits.
        acl_probe = subprocess.run(["/usr/bin/setfacl", "--set=u::rwx,g::r-x,g:2999:r-x,m::r-x,o::--x", str(original_slice)],
                                   capture_output=True, text=True)
        evidence["extended_cgroup_acl_supported"] = acl_probe.returncode == 0
        if acl_probe.returncode != 0:
            helper.command("setfacl", "--set=u::rwx,g::r-x,o::--x", original_slice)
        original_slice.chmod(0o2751)
        (original_slice / "cgroup.max.descendants").write_text("77")
        baseline_holder = launch(["/usr/bin/sleep", "infinity"], "baseline-holder", 2001)
        wait_for(lambda: (original_slice / "fc-test-baseline-holder.scope").exists(), "baseline login session")
        baseline_acl = helper.command("getfacl", "--omit-header", "--numeric", original_slice)
        baseline_gid = original_slice.stat().st_gid
        baseline_mode = original_slice.stat().st_mode & 0o7777
        baseline_limits = {name: (original_slice / name).read_text() for name in
                           ("cpu.max", "memory.high", "memory.max", "memory.swap.max", "pids.max", "cgroup.max.descendants")}
        manual_before = environment_state()
        manual_args = [sys.executable, LAUNCHER]
        for number in range(1, 4):
            manual_args += ["--player", f"fearless_p{number}"]
        manual_args += ["--", "--manual", "--scenario", "--preset", "demo3"]
        manual = launch(manual_args, "manual-existing")
        for _ in range(4):
            send(manual, "")
        wait_for(lambda: socket.exists() or manual.poll() is not None, "existing-account startup")
        check("existing-account mode starts normally", socket.exists() and manual.poll() is None, log("manual-existing"))
        before_users = Path("/etc/passwd").read_bytes()
        overlap = subprocess.run([sys.executable, LAUNCHER, "--players", "3", "--socket", "/run/fearless-conspiracy/other.sock"], capture_output=True, text=True)
        check("overlapping game cannot delete active participants", overlap.returncode != 0 and "already running" in overlap.stderr and Path("/etc/passwd").read_bytes() == before_users)
        send(manual, "shutdown")
        manual.wait(timeout=20)
        check("existing-account mode preserves accounts and home data", manual.returncode == 0 and Path("/etc/passwd").read_bytes() == before_users and Path("/home/fearless_p1/keep-me").read_text() == "player notes")
        restored("existing-account shutdown preserves nondefault proc and administrator overrides", manual_before)
        check("active slice limits ownership ACL and existing session are restored",
              baseline_holder.poll() is None and original_slice.stat().st_gid == baseline_gid
              and original_slice.stat().st_mode & 0o7777 == baseline_mode
              and helper.command("getfacl", "--omit-header", "--numeric", original_slice) == baseline_acl
              and all((original_slice / name).read_text() == value for name, value in baseline_limits.items()))
        helper.command("systemctl", "stop", "fc-test-baseline-holder.scope")
        baseline_holder.wait(timeout=20)

        for signum, label in ((signal.SIGTERM, "terminate"), (signal.SIGINT, "interrupt"), (signal.SIGHUP, "hangup")):
            signal_before = environment_state()
            interrupted = launch([sys.executable, LAUNCHER, "--players", "3", "--", "--manual", "--scenario", "--preset", "demo3"], label)
            for _ in range(4):
                send(interrupted, "")
            wait_for(lambda: socket.exists() or interrupted.poll() is not None, label + " startup", timeout=90)
            check(label + " test starts normally", socket.exists() and interrupted.poll() is None, log(label))
            interrupted.send_signal(signum)
            interrupted.wait(timeout=30)
            check(label + " cleans accounts homes groups credentials and socket", interrupted.returncode == 128 + signum and players_removed(3) and not socket.exists() and not (state / "player-logins.txt").exists(), {"exit": interrupted.returncode, "log": log(label)[-700:]})
            restored(label + " restores prior isolation settings", signal_before)
        failed_before = environment_state()
        failed = launch([sys.executable, LAUNCHER, "--players", "3", "--", "--not-a-game-option"], "failed-game")
        failed.wait(timeout=60)
        check("failed game startup cleans provisioned players", failed.returncode != 0 and players_removed(3) and not socket.exists()
              and not (state / "player-logins.txt").exists(), {"exit": failed.returncode, "log": log("failed-game")[-700:]})
        restored("failed game startup restores prior isolation settings", failed_before)

        target = OUT / "unrelated-config"
        target.write_text("[Slice]\nMemoryMax=400M\n")
        dropins = helper.OVERRIDES / "user-2002.slice.d"
        dropins.mkdir(mode=0o755, parents=True, exist_ok=True)
        unsafe = dropins / "50-MemoryMax.conf"
        unsafe.symlink_to(target)
        partial_before = environment_state()
        partial = launch([sys.executable, LAUNCHER, "--players", "3"], "partial-setup")
        partial.wait(timeout=60)
        check("mid-setup failure safely removes temporary players", partial.returncode != 0 and players_removed(3)
              and "unsafe runtime override" in log("partial-setup") and target.read_text() == "[Slice]\nMemoryMax=400M\n")
        restored("mid-setup failure restores earlier changed slices and proc", partial_before)
        unsafe.unlink()
        dropins.rmdir()

        killed_before = environment_state()
        killed = launch([sys.executable, LAUNCHER, "--players", "3", "--", "--manual", "--scenario", "--preset", "demo3"], "killed-launcher")
        for _ in range(4):
            send(killed, "")
        wait_for(lambda: socket.exists() or killed.poll() is not None, "forced-termination startup", timeout=90)
        check("forced-termination game starts normally", socket.exists() and killed.poll() is None)
        killed.kill()
        killed.wait(timeout=20)
        check("SIGKILL retains a root-only recovery record", (state / "isolation.json").stat().st_mode & 0o777 == 0o600)
        recovery = subprocess.run(["/bin/bash", "/lab/server", "--restore"], capture_output=True, text=True, timeout=45)
        check("explicit recovery stops orphaned game and removes managed accounts", recovery.returncode == 0
              and players_removed(3) and not socket.exists(), recovery.stdout + recovery.stderr)
        restored("SIGKILL recovery restores original settings", killed_before)
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
