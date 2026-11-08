#!/usr/bin/python3
"""Privileged setup for a protected game on a dedicated systemd/cgroup-v2 VM."""
import argparse
import ctypes
import grp
import os
from pathlib import Path
import pwd
import re
import shutil
import stat
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CGROUP = Path("/sys/fs/cgroup")
RUNTIME = Path("/run/fearless-conspiracy")
ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C", "LC_ALL": "C"}
PLAYER_LIMITS = {"CPUQuota": "25%", "MemoryHigh": "192M", "MemoryMax": "256M", "MemorySwapMax": "0", "TasksMax": "64"}
SERVER_LIMITS = {"CPUQuota": "100%", "MemoryHigh": "384M", "MemoryMax": "512M", "MemorySwapMax": "0", "TasksMax": "128"}
ADMIN_GROUPS = {"root", "sudo", "wheel", "admin", "docker", "lxd", "incus-admin", "disk", "adm", "systemd-journal", "shadow"}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def command(program, *args):
    path = Path("/usr/bin") / program
    info = path.stat()
    require(info.st_uid == 0 and not info.st_mode & 0o022,
            f"{path} must be root-owned and not writable by other accounts")
    return subprocess.run([str(path), *map(str, args)], check=True, capture_output=True,
                          text=True, env=ENV, cwd="/").stdout.strip()


def account(name):
    entry = pwd.getpwnam(name)
    require(entry.pw_uid != 0 and entry.pw_gid != 0, f"{name} must be a non-root account with a private group")
    return entry


def groups(entry):
    return set(os.getgrouplist(entry.pw_name, entry.pw_gid))


def sudo_denies(result, name):
    # sudo-rs returns 0 for an empty privilege listing; traditional sudo may
    # return 1. Require an explicit denial without errors, not an exit code.
    denial = rf"User {re.escape(name)} is not allowed to run sudo on [^\r\n]+\."
    stdout, stderr = result.stdout.strip(), result.stderr.strip()
    return result.returncode in (0, 1) and bool(
        (re.fullmatch(denial, stdout) and not stderr)
        or (re.fullmatch(denial, stderr) and not stdout))


def validate_accounts(server, players):
    require(3 <= len(players) <= 9, "register 3-9 player accounts with --player")
    accounts = [server, *players]
    require(len({entry.pw_uid for entry in accounts}) == len(accounts), "server and players must have distinct Linux UIDs")
    require(len({entry.pw_gid for entry in accounts}) == len(accounts), "server and players need distinct private primary groups")
    for player in [*players, server]:
        membership = groups(player)
        require(player == server or not any(entry.pw_gid in membership for entry in accounts if entry.pw_uid != player.pw_uid),
                f"{player.pw_name} belongs to another game account's private group")
        names = {grp.getgrgid(gid).gr_name for gid in membership}
        require(not names & ADMIN_GROUPS, f"{player.pw_name} belongs to a privileged group: {sorted(names & ADMIN_GROUPS)}")
        if Path("/usr/bin/sudo").exists():
            result = subprocess.run(["/usr/bin/sudo", "-n", "-l", "-U", player.pw_name],
                                    capture_output=True, text=True, env=ENV, cwd="/")
            require(sudo_denies(result, player.pw_name),
                    f"{player.pw_name} must have no sudo permissions; policy check: {(result.stdout + result.stderr).strip()}")


def proc_status(pid):
    return dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines() if ":" in line)


def validate_sessions(players):
    """Reject already-running sessions with different mounts or extra powers."""
    uids = {entry.pw_uid for entry in players}
    memberships = {entry.pw_uid: groups(entry) for entry in players}
    namespace = os.stat("/proc/self/ns/mnt").st_ino
    require(namespace == os.stat("/proc/1/ns/mnt").st_ino,
            "run the launcher in the VM's host mount namespace")
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            fields = proc_status(directory.name)
            identities = set(map(int, fields["Uid"].split()))
            if not identities & uids:
                continue
            require(len(identities) == 1 and not int(fields["CapEff"], 16)
                    and not int(fields["CapPrm"], 16) and not int(fields["CapAmb"], 16),
                    f"player process {directory.name} has elevated credentials")
            uid = next(iter(identities))
            process_groups = set(map(int, (fields["Gid"] + " " + fields["Groups"]).split()))
            require(process_groups <= memberships[uid], f"player process {directory.name} has unexpected group privileges; log it out first")
            prefix = f"0::/user.slice/user-{uid}.slice"
            require(any(line == prefix or line.startswith(prefix + "/")
                        for line in (directory / "cgroup").read_text().splitlines()),
                    f"player process {directory.name} is outside its systemd user slice; log it out and log in normally")
            require(os.stat(directory / "ns/mnt").st_ino == namespace,
                    f"player process {directory.name} uses a different /proc view; log it out first")
        except FileNotFoundError:
            continue  # Process exited during inspection.


def restrict_proc():
    mounts = [line.split() for line in Path("/proc/self/mountinfo").read_text().splitlines()]
    mount = next((fields for fields in reversed(mounts) if fields[4] == "/proc"), None)
    require(mount is not None and mount[mount.index("-") + 1] == "proc", "/proc must be a procfs mount")
    # Keep the current VFS security/atime flags while changing procfs options.
    flags = 32  # MS_REMOUNT
    values = {"ro": 1, "nosuid": 2, "nodev": 4, "noexec": 8, "sync": 16,
              "noatime": 1024, "nodiratime": 2048, "relatime": 1 << 21,
              "strictatime": 1 << 24, "lazytime": 1 << 25}
    for option in mount[5].split(","):
        flags |= values.get(option, 0)
    libc = ctypes.CDLL(None, use_errno=True)
    libc.mount.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_ulong, ctypes.c_char_p]
    require(libc.mount(None, b"/proc", None, flags, b"hidepid=2,gid=0") == 0,
            f"cannot restrict /proc: {os.strerror(ctypes.get_errno())}")


def configure_slice(entry, limits):
    unit = f"user-{entry.pw_uid}.slice"
    command("systemctl", "set-property", "--runtime", unit, *(f"{key}={value}" for key, value in limits.items()))
    command("systemctl", "start", unit)
    path = command("systemctl", "show", "--property=ControlGroup", "--value", unit)
    require(path == f"/user.slice/{unit}", f"unexpected cgroup for {unit}: {path}")
    directory = CGROUP / path.lstrip("/")
    require(directory.stat().st_uid == 0, f"{directory} must be root-owned")
    # Each account can traverse its own slice; only the trusted server also
    # receives all player private GIDs. Remove any old extended ACL entries.
    os.chown(directory, 0, entry.pw_gid)
    command("setfacl", "--set=u::rwx,g::r-x,o::---", directory)
    (directory / "cgroup.max.descendants").write_text("256")
    for key, filename in (("MemoryHigh", "memory.high"), ("MemoryMax", "memory.max"),
                          ("MemorySwapMax", "memory.swap.max"), ("TasksMax", "pids.max")):
        value = limits[key]
        expected = str(int(value[:-1]) * 1024 * 1024) if value.endswith("M") else value
        require((directory / filename).read_text().strip() == expected, f"kernel did not apply {key} for {entry.pw_name}")
    quota, period = (directory / "cpu.max").read_text().split()
    require(quota != "max" and int(quota) * 100 == int(period) * int(limits["CPUQuota"].rstrip("%")),
            f"kernel did not apply CPUQuota for {entry.pw_name}")
    return directory


PROBE = r'''
import json, os, sys
from pathlib import Path
pid, own, others = sys.argv[1], Path(sys.argv[2]), sys.argv[3:]
for path in [f"/proc/{pid}/task", f"/proc/{pid}/status", *others]:
    try:
        p = Path(path)
        list(p.iterdir()) if p.is_dir() else p.read_text()
    except (FileNotFoundError, PermissionError):
        pass
    else:
        raise SystemExit("private metadata remains readable: " + path)
assert Path("/proc/self/status").read_text()
assert (own / "cpu.stat").read_text()
print(json.dumps({"uid": os.getuid(), "proc_hidden": True, "other_cgroups_hidden": True}))
'''


def unprivileged(entry, extra_groups=()):
    return ["/usr/bin/setpriv", "--reuid", str(entry.pw_uid), "--regid", str(entry.pw_gid),
            "--groups", ",".join(map(str, sorted(groups(entry) | set(extra_groups)))),
            "--no-new-privs", "--bounding-set=-all", "--inh-caps=-all", "--ambient-caps=-all"]


def stage_binary(source, directory):
    """Copy data without executing it as root; replace without following links."""
    with source.open("rb") as incoming:
        require(incoming.read(4) == b"\x7fELF", "the prebuilt server must be a Linux ELF executable")
        incoming.seek(0)
        fd, temporary = tempfile.mkstemp(prefix=".server-", dir=directory)
        try:
            with os.fdopen(fd, "wb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)
                os.fchmod(outgoing.fileno(), 0o555)
            destination = directory / "fearless-conspiracy-server"
            os.replace(temporary, destination)
            return destination
        finally:
            Path(temporary).unlink(missing_ok=True)


def verify_visibility(server, players, slices):
    with subprocess.Popen(["/usr/bin/sleep", "infinity"], user=server.pw_uid, group=server.pw_gid, extra_groups=[], env=ENV) as sentinel:
        try:
            for player in players:
                require(sentinel.poll() is None, "the process-visibility probe exited unexpectedly")
                others = [str(path / "cpu.stat") for uid, path in slices.items() if uid != player.pw_uid]
                result = subprocess.run([*unprivileged(player), "/usr/bin/python3", "-I", "-c", PROBE,
                                         str(sentinel.pid), str(slices[player.pw_uid]), *others],
                                        check=True, capture_output=True, text=True, env=ENV, cwd="/")
                print("Verified player isolation:", result.stdout.strip(), flush=True)
            require(sentinel.poll() is None, "the process-visibility probe exited unexpectedly")
        finally:
            sentinel.terminate()
            sentinel.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-user", default="fearless_host")
    parser.add_argument("--player", action="append", required=True, metavar="ACCOUNT")
    parser.add_argument("--socket", type=Path, default=RUNTIME / "game.sock")
    parser.add_argument("game_args", nargs=argparse.REMAINDER, help="game options after --")
    args = parser.parse_args()
    require(os.geteuid() == 0, "protected startup requires sudo; build the binaries first as your normal user")
    require(Path("/run/systemd/system").is_dir() and (CGROUP / "cgroup.controllers").exists(),
            "protected startup requires systemd and cgroup v2")
    require({"cpu", "memory", "pids"} <= set((CGROUP / "cgroup.controllers").read_text().split()),
            "the CPU, memory and pids cgroup controllers must be available")
    for program in ("systemctl", "systemd-run", "setpriv", "setfacl"):
        info = (Path("/usr/bin") / program).stat()
        require(info.st_uid == 0 and not info.st_mode & 0o022, f"install a trusted /usr/bin/{program}")
    server = account(args.server_user)
    players = [account(name) for name in args.player]
    validate_accounts(server, players)
    validate_sessions(players)
    for fd in (1, 2):
        info = os.fstat(fd)
        require(not stat.S_ISREG(info.st_mode) or not info.st_mode & 0o077,
                "moderator output files must be private (mode 0600); use umask 077 before redirecting output")
    game_args = args.game_args[1:] if args.game_args[:1] == ["--"] else args.game_args
    require(not any(value in {"--allow-same-uid", "--protected-users"} for value in game_args),
            "protected startup controls the account restrictions")
    binary = next((path for path in [ROOT / "bin/fearless-conspiracy-server",
                                    ROOT / "target/debug/fearless-conspiracy-server"] if path.is_file()), None)
    require(binary is not None, "build with cargo build --locked --bins before running sudo")
    socket = args.socket.absolute()
    require(socket.parent == RUNTIME and socket.name not in {".", ".."}, f"the protected socket must be inside {RUNTIME}")
    require(not socket.exists() and not socket.is_symlink(), f"socket path already exists: {socket}")
    if RUNTIME.exists() or RUNTIME.is_symlink():
        info = RUNTIME.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid in (0, server.pw_uid), f"unsafe runtime directory: {RUNTIME}")
    else:
        RUNTIME.mkdir(mode=0o755)
    os.chown(RUNTIME, server.pw_uid, server.pw_gid)
    RUNTIME.chmod(0o755)
    os.umask(0o077)
    # A dedicated server account need not have access to the moderator's home
    # directory. Stage the built executable without building/running it as root.
    binary = stage_binary(binary, RUNTIME)
    print("Applying VM-wide /proc protection and runtime account limits; these remain until reboot.", flush=True)
    restrict_proc()
    slices = {entry.pw_uid: configure_slice(entry, SERVER_LIMITS if entry == server else PLAYER_LIMITS)
              for entry in [server, *players]}
    verify_visibility(server, players, slices)
    # systemd places the process into the server's bounded slice before setpriv
    # permanently drops credentials/capabilities and execs the game binary.
    cmd = ["/usr/bin/systemd-run", "--quiet", "--scope", f"--unit=fearless-conspiracy-{os.getpid()}",
           f"--slice=user-{server.pw_uid}.slice", *unprivileged(server, [entry.pw_gid for entry in players]),
           str(binary), str(socket), "--players", str(len(players)), "--shared-local", "--setup",
           "--protected-users", ",".join(str(entry.pw_uid) for entry in players), *game_args]
    env = {**ENV, "HOME": server.pw_dir, "USER": server.pw_name, "LOGNAME": server.pw_name}
    os.chdir("/")
    os.execve(cmd[0], cmd, env)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, KeyError, subprocess.CalledProcessError) as exc:
        print(f"Protected startup refused: {exc}", file=sys.stderr)
        if isinstance(exc, subprocess.CalledProcessError) and exc.stderr:
            print(exc.stderr.strip(), file=sys.stderr)
        sys.exit(1)
