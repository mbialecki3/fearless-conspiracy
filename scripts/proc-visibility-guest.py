#!/usr/bin/env python3
"""VM-only controller and unprivileged /proc observer. See run-proc-visibility-vm.py."""
import errno
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import traceback


def inspect(pid):
    tasks = {}
    for task in sorted(Path(f"/proc/{pid}/task").iterdir()):
        status = (task / "status").read_text()
        fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
        schedstat = (task / "schedstat").read_text()
        runtime, wait, slices = map(int, schedstat.split())
        tasks[task.name] = {
            "name": (task / "comm").read_text().strip(),
            "runtime_ns": runtime, "wait_ns": wait, "timeslices": slices,
            "voluntary_switches": int(fields["voluntary_ctxt_switches"]),
            "switches": int(fields["voluntary_ctxt_switches"]) + int(fields["nonvoluntary_ctxt_switches"]),
            "raw_status": status, "raw_schedstat": schedstat,
        }
    return tasks


def observer(pid, known_tid, other_pid):
    """No roles, transcripts, or sockets are accessible to this process."""
    def attempt(operation):
        try:
            return {"ok": True, "value": operation()}
        except OSError as exc:
            return {"ok": False, "errno": exc.errno, "error": exc.strerror}
    return {
        "uid": os.getuid(), "gid": os.getgid(), "groups": os.getgroups(),
        "self_status": Path("/proc/self/status").read_text(),
        "proc_mount": [line for line in Path("/proc/mounts").read_text().splitlines() if line.split()[1] == "/proc"],
        "server_listed": str(pid) in os.listdir("/proc"),
        "server_tasks": attempt(lambda: inspect(pid)),
        "server_cmdline": attempt(lambda: Path(f"/proc/{pid}/cmdline").read_text().replace("\0", " ")),
        "server_status": attempt(lambda: Path(f"/proc/{pid}/status").read_text()),
        "server_wchan": attempt(lambda: Path(f"/proc/{pid}/wchan").read_text()),
        "server_environ": attempt(lambda: Path(f"/proc/{pid}/environ").read_text()),
        "server_fd": attempt(lambda: os.listdir(f"/proc/{pid}/fd")),
        "known_tid_status": attempt(lambda: Path(f"/proc/{known_tid}/status").read_text()),
        "other_player_status": attempt(lambda: Path(f"/proc/{other_pid}/status").read_text()),
        "own_tasks": attempt(lambda: inspect(os.getpid())),
        "ps_threads": subprocess.run(["/bin/busybox", "ps", "-T"], capture_output=True, text=True).stdout,
    }


def active(before, after, tid):
    # Predeclared threshold, verified against idle controls. Background wakeups
    # make a zero/nonzero classifier unreliable on this kernel under TCG.
    return after[tid]["voluntary_switches"] - before[tid]["voluntary_switches"] >= 4


def main():
    if not Path("/FC_DISPOSABLE_VM").exists() or Path("/sys/class/dmi/id/product_name").read_text().strip() != "FC proc visibility disposable VM":
        raise SystemExit("Refusing: this controller may run only inside the dedicated test VM")
    out = Path("/results")
    out.mkdir(exist_ok=True, mode=0o700)
    os.chmod(out, 0o700)
    evidence = {"kernel": os.uname().release, "machine": os.uname().machine,
                "isolation": "QEMU TCG dedicated VM, no host mounts, no network",
                "accounts": {"fc_server": 2000, **{f"fc_p{p}": 2000+p for p in range(1, 10)}},
                "observer_inputs": "kernel metadata plus public serial join and ready calibration; oracle attached after inference",
                "classifier": "five spaced commands per window; active iff voluntary_ctxt_switches delta >= 4; equal-duration idle controls",
                "snapshots": {}, "measurements": []}
    processes, handles = [], []

    def wait_for(condition, description, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if condition():
                return
            time.sleep(0.05)
        raise RuntimeError(f"timeout: {description}")

    def send(process, command):
        process.stdin.write(command + "\n")
        process.stdin.flush()

    def log(label):
        return (out / f"{label}.log").read_text()

    def launch(binary, args, label, uid):
        handle = (out / f"{label}.log").open("w")
        handles.append(handle)
        process = subprocess.Popen([f"/lab/{binary}", *map(str, args)], stdin=subprocess.PIPE,
                                   stdout=handle, stderr=subprocess.STDOUT, text=True,
                                   user=uid, group=uid, extra_groups=[], umask=0o077, cwd="/tmp")
        processes.append(process)
        return process

    observer_uid, known_tid, other_pid = 2001, 1, 1

    def probe(label, uid=None, groups=(), detailed=False):
        result = subprocess.run([sys.executable, __file__, "--observe" if detailed else "--snapshot", str(server.pid), str(known_tid), str(other_pid)],
                                user=observer_uid if uid is None else uid, group=observer_uid if uid is None else uid,
                                extra_groups=list(groups), capture_output=True, text=True, check=True, cwd="/tmp")
        value = json.loads(result.stdout)
        evidence["snapshots"][label] = value
        return value

    try:
        os.mkdir("/tmp/game", 0o755)
        os.chown("/tmp/game", 2000, 2000)
        socket = Path("/tmp/game/game.sock")
        server = launch("fearless-conspiracy-server", [socket, "--preset", "full9", "--manual", "--shared-local"], "server", 2000)
        evidence["server_pid"] = server.pid
        wait_for(socket.exists, "server socket")
        clients, pairs = {}, {}
        for player in range(1, 10):
            before = probe(f"join_{player}_before")["server_tasks"]["value"]
            clients[player] = launch("fearless-conspiracy-client", [socket, f"Player{player}", "--plain"], f"player{player}", 2000+player)
            other_pid = clients[player].pid
            wait_for(lambda p=player: f"#{p} Player{p} joined" in log("server"), f"join {player}")
            time.sleep(0.4)
            after = probe(f"join_{player}_after")["server_tasks"]["value"]
            pair = sorted(set(after)-set(before), key=int)
            assert len(pair) == 2, pair
            pairs[player] = pair
        evidence["thread_pairs_from_public_serial_joins"] = pairs

        def measure(label, command=None, sender=None):
            time.sleep(0.5)
            before = probe(label + "_before")["server_tasks"]
            for _ in range(5):
                if command is not None:
                    send(clients[sender], command)
                time.sleep(0.3)
            time.sleep(0.7)
            after = probe(label + "_after")["server_tasks"]
            return before, after

        first = measure("ready_1", "/ready", 1)
        second = measure("ready_2", "/ready", 2)
        calibrated = {}
        for player, pair in pairs.items():
            before, after = second if player == 1 else first
            candidates = [tid for tid in pair if active(before["value"], after["value"], tid)]
            # Both socket reader and writer can wake for outgoing traffic.
            # Mapping the connection's pair is sufficient; never assume that
            # only its writer wakes or assign orientation from role knowledge.
            assert candidates, (player, candidates)
            calibrated[player] = candidates
        evidence["active_connection_tids_from_public_broadcasts"] = calibrated
        known_tid = int(pairs[1][0])
        for player in range(3, 10):
            send(clients[player], "/ready")
            time.sleep(0.2)
        wait_for(lambda: all("You are the " in log(f"player{p}") for p in clients), "roles")
        roles = {}
        for player in clients:
            match = re.search(r"You are the (.+?) \((Town|Mafia|Neutral)\)", log(f"player{player}"))
            assert match, player
            roles[player] = {"role": match[1], "faction": match[2]}
        mafia = sorted(p for p in roles if roles[p]["faction"] == "Mafia")
        jailor = next(p for p in roles if roles[p]["role"] == "Jailor")
        prisoner = next(p for p in roles if roles[p]["role"] == "Sheriff")
        town = next(p for p in roles if roles[p]["role"] == "Lookout")
        observer_uid = 2000 + town
        other_pid = clients[1 if town != 1 else 2].pid
        evidence["town_observer_player"] = town
        evidence["process_uids_root_validation_only"] = {str(p.pid): re.search(r"^Uid:.*$", Path(f"/proc/{p.pid}/status").read_text(), re.M).group() for p in processes}
        send(server, "advance")
        wait_for(lambda: "Day 1 Introductions" in log("server"), "day")
        send(clients[jailor], f"/jail {prisoner}")
        wait_for(lambda: "You will jail" in log(f"player{jailor}"), "jail selection")
        send(server, "advance")
        wait_for(lambda: "Night 1 has begun" in log(f"player{town}"), "night")

        def record(label, command=None, sender=None, expected=(), hidden=False):
            before, after = measure(label, command, sender)
            row = {"label": label, "metadata_readable": before["ok"] and after["ok"]}
            if row["metadata_readable"]:
                row["inferred_active_recipients"] = sorted(p for p, tids in pairs.items() if any(active(before["value"], after["value"], tid) for tid in tids))
                row["connection_thread_deltas"] = {p: {tid: {key: after["value"][tid][key]-before["value"][tid][key] for key in ("runtime_ns", "timeslices", "switches", "voluntary_switches")} for tid in tids} for p, tids in pairs.items()}
            else:
                row["inferred_active_recipients"] = None
            # Oracle is attached only after the observer's inference above.
            row["expected_recipients_validation_only"] = sorted(expected)
            row["pass"] = (not row["metadata_readable"]) if hidden else row["inferred_active_recipients"] == sorted(expected)
            evidence["measurements"].append(row)
            print(json.dumps(row), flush=True)
            assert row["pass"], row

        for mode in (0, 2, 0):
            subprocess.run(["/bin/busybox", "mount", "-o", f"remount,hidepid={mode}", "/proc"], check=True)
            prefix = f"hidepid{mode}" if mode == 2 or not evidence["measurements"] else "restored0"
            visibility = probe(prefix + "_visibility", detailed=True)
            if mode == 2:
                assert not visibility["server_listed"]
                for key in ("server_tasks", "server_cmdline", "server_status", "known_tid_status", "other_player_status"):
                    assert not visibility[key]["ok"] and visibility[key]["errno"] in (errno.ENOENT, errno.EACCES), key
                assert visibility["own_tasks"]["ok"]
                assert probe("hidepid2_server_account_control", uid=2000, detailed=True)["server_tasks"]["ok"]
                assert probe("hidepid2_root_control", uid=0, detailed=True)["server_tasks"]["ok"]
            else:
                assert visibility["server_tasks"]["ok"]
            record(prefix + "_idle", hidden=mode == 2)
            for number, sender in enumerate(mafia, 1):
                token = f"secretprobe-{prefix}-{number}"
                record(prefix + f"_mafia_{number}", "/mafia " + token, sender, mafia, hidden=mode == 2)
                wait_for(lambda t=token: all(t in log(f"player{p}") for p in mafia), "mafia delivery")
                assert token not in log(f"player{town}")
            record(prefix + "_town_reply", "/players", town, [town], hidden=mode == 2)
            if prefix != "restored0":
                for number, sender in enumerate((jailor, prisoner), 1):
                    token = f"jailprobe-{prefix}-{number}"
                    record(prefix + f"_jail_{number}", "/jailchat " + token, sender, [jailor, prisoner], hidden=mode == 2)
                    wait_for(lambda t=token: all(t in log(f"player{p}") for p in (jailor, prisoner)), "jail delivery")
                    assert token not in log(f"player{town}")
            record(prefix + "_idle_after", hidden=mode == 2)
        # Demonstrate that the mount's exempt group restores the leak for members.
        subprocess.run(["/bin/busybox", "mount", "-o", "remount,hidepid=2,gid=2999", "/proc"], check=True)
        assert not probe("hidepid2_gid_nonmember", detailed=True)["server_tasks"]["ok"]
        assert probe("hidepid2_gid_member", groups=(2999,), detailed=True)["server_tasks"]["ok"]
        evidence["ground_truth_validation_only"] = roles
        evidence["town_received_secret_probes"] = "secretprobe-" in log(f"player{town}") or "jailprobe-" in log(f"player{town}")
        assert not evidence["town_received_secret_probes"]
        send(server, "shutdown")
        server.wait(timeout=15)
        evidence["pass"] = True
    except BaseException:
        evidence["pass"] = False
        evidence["error"] = traceback.format_exc()
        traceback.print_exc()
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.kill()
            process.wait()
        for handle in handles:
            handle.close()
        (out / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    return 0 if evidence["pass"] else 1


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--observe":
        print(json.dumps(observer(*map(int, sys.argv[2:]))))
    elif len(sys.argv) > 1 and sys.argv[1] == "--snapshot":
        try:
            tasks = {"ok": True, "value": inspect(int(sys.argv[2]))}
        except OSError as exc:
            tasks = {"ok": False, "errno": exc.errno, "error": exc.strerror}
        print(json.dumps({"uid": os.getuid(), "groups": os.getgroups(), "server_tasks": tasks}))
    else:
        sys.exit(main())
