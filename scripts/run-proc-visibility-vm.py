#!/usr/bin/env python3
"""Boot a disposable QEMU VM for /proc tests; never mount/remount host /proc.

Linux host requirements: cargo, Python 3, busybox (static), cpio, gzip, ldd,
QEMU x86 and a readable Linux bzImage. No sudo/KVM/network/host share required.
QEMU packages may be extracted locally with apt-get download + dpkg-deb -x.
"""
import argparse
import base64
import datetime
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import sysconfig
import tarfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", type=Path, required=True)
    parser.add_argument("--qemu-share", type=Path, required=True)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--bios", type=Path, help="SeaBIOS path when using locally extracted QEMU packages")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--protected-launcher", action="store_true", help="test protected startup with real systemd and cgroup v2")
    args = parser.parse_args()
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = (args.output or ROOT / "target/proc-vm" / stamp).resolve()
    out.mkdir(parents=True, exist_ok=False)
    initroot = out / "initroot"
    initroot.mkdir()
    subprocess.run(["cargo", "build", "--locked", "--bins"], cwd=ROOT, check=True)

    def copy_file(source, target=None):
        source = Path(source)
        dest = initroot / str(target or source).lstrip("/")
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest, follow_symlinks=True)
        return dest

    def copy_libs(binary):
        result = subprocess.run(["ldd", str(binary)], capture_output=True, text=True)
        for dependency in re.findall(r"(?:=>\s+)?(/[^\s()]+)", result.stdout):
            copy_file(dependency)

    for directory in ("dev", "proc", "sys", "tmp", "etc", "results", "lab", "bin", "usr/bin"):
        (initroot / directory).mkdir(parents=True, exist_ok=True)
    os.chmod(initroot / "tmp", 0o1777)
    busybox = shutil.which("busybox")
    if "not a dynamic executable" not in subprocess.run(["ldd", busybox], capture_output=True, text=True).stderr:
        raise SystemExit("Use a static busybox to boot the initramfs")
    copy_file(busybox, "/bin/busybox")
    (initroot / "bin/sh").symlink_to("busybox")
    (initroot / "bin/gzip").symlink_to("busybox")
    copy_file(sys.executable, "/usr/bin/python3")
    copy_libs(sys.executable)
    stdlib = Path(sysconfig.get_path("stdlib"))
    shutil.copytree(stdlib, initroot / str(stdlib).lstrip("/"),
                    ignore=shutil.ignore_patterns("__pycache__", "site-packages", "dist-packages", "test", "tests", "idlelib", "tkinter", "ensurepip"))
    for extension in (stdlib / "lib-dynload").glob("*.so"):
        copy_libs(extension)
    for name in ("fearless-conspiracy-server", "fearless-conspiracy-client"):
        binary = ROOT / "target/debug" / name
        copy_file(binary, "/lab/" + name)
        copy_libs(binary)
    guest = "protected-server-guest.py" if args.protected_launcher else "proc-visibility-guest.py"
    copy_file(ROOT / "scripts" / guest, "/lab/guest.py")
    (initroot / "FC_DISPOSABLE_VM").touch()
    accounts = [("root", 0), ("fc_server", 2000), *[(f"fc_p{p}", 2000+p) for p in range(1, 10)]]
    (initroot / "etc/passwd").write_text("".join(f"{name}:x:{uid}:{uid}::/tmp:/bin/sh\n" for name, uid in accounts))
    (initroot / "etc/group").write_text("".join(f"{name}:x:{uid}:\n" for name, uid in accounts) + "proc_observers:x:2999:\n")
    if args.protected_launcher:
        accounts = [("root", 0)]  # Protected startup must create the actual game accounts.
        (initroot / "etc/passwd").write_text("".join(f"{name}:x:{uid}:{uid}::/tmp:/bin/sh\n" for name, uid in accounts))
        (initroot / "etc/group").write_text("".join(f"{name}:x:{uid}:\n" for name, uid in accounts))
        (initroot / "etc/shadow").write_text("root:!:0:0:99999:7:::\n")
        (initroot / "etc/shadow").chmod(0o600)
        (initroot / "etc/gshadow").write_text("root:!::\n")
        (initroot / "etc/gshadow").chmod(0o600)
        (initroot / "etc/login.defs").write_text("UID_MIN 2000\nUID_MAX 60000\nGID_MIN 2000\nGID_MAX 60000\nENCRYPT_METHOD SHA512\n")
        for directory in ("var/lib", "home", "etc/skel"):
            (initroot / directory).mkdir(parents=True, exist_ok=True)
        (initroot / "etc/default").mkdir(exist_ok=True)
        (initroot / "etc/default/useradd").write_text("CREATE_MAIL_SPOOL=no\n")
        (initroot / "etc/machine-id").write_text("12345678123456781234567812345678\n")
        copy_file("/etc/os-release")
        for name in ("systemctl", "systemd-run", "setpriv", "setfacl", "getfacl", "sleep", "true", "sudo", "dirname"):
            binary = Path("/usr/bin") / name
            copy_file(binary)
            copy_libs(binary)
        for binary in [Path("/usr/sbin") / name for name in ("useradd", "userdel", "usermod", "groupadd", "groupdel", "chpasswd", "nologin")] + [Path("/bin/bash")]:
            copy_file(binary)
            copy_libs(binary)
        for plugin in Path("/usr/libexec/sudo").glob("*.so*"):
            copy_file(plugin)
            copy_libs(plugin)
        (initroot / "etc/sudoers").write_text("root ALL=(ALL:ALL) ALL\n")
        (initroot / "etc/sudoers").chmod(0o440)
        (initroot / "etc/pam.d").mkdir()
        (initroot / "etc/pam.d/sudo").write_text("auth sufficient pam_rootok.so\naccount required pam_permit.so\nsession required pam_permit.so\n")
        for name in ("pam_rootok.so", "pam_permit.so"):
            plugin = Path("/usr/lib/x86_64-linux-gnu/security") / name
            copy_file(plugin)
            copy_libs(plugin)
        for name in ("systemd", "systemd-executor"):
            binary = Path("/usr/lib/systemd") / name
            copy_file(binary)
            copy_libs(binary)
        # systemd loads these libraries at runtime, so ldd does not list them.
        for name in ("libmount.so.1", "libblkid.so.1", "libkmod.so.2", "libudev.so.1", "libcap.so.2", "libacl.so.1"):
            library = Path("/usr/lib/x86_64-linux-gnu") / name
            copy_file(library)
            copy_libs(library)
        for name in ("protected-server.py", "test-protected-server.py"):
            copy_file(ROOT / "scripts" / name, "/lab/scripts/" + name)
        copy_file(ROOT / "server", "/lab/server")
        for name in ("fearless-conspiracy-server", "fearless-conspiracy-client"):
            copy_file(ROOT / "target/debug" / name, "/lab/bin/" + name)
        (initroot / "etc/systemd/system").mkdir(parents=True)
        (initroot / "etc/systemd/system.conf").write_text("[Manager]\nLogLevel=warning\nShowStatus=no\n")
        (initroot / "etc/systemd/system/fc-test.target").write_text("[Unit]\nDefaultDependencies=no\nWants=fc-test.service\n")
        (initroot / "etc/systemd/system/fc-test.service").write_text(
            "[Unit]\nDefaultDependencies=no\n[Service]\nType=simple\nExecStart=/bin/sh /lab/export-test.sh\nStandardOutput=tty\nStandardError=tty\nTTYPath=/dev/console\n")
    init = initroot / "init"
    init.write_text('''#!/bin/sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
/bin/busybox chmod 1777 /tmp
/bin/busybox chmod 700 /results
export PATH=/usr/bin:/bin
export PYTHONUNBUFFERED=1
/usr/bin/python3 /lab/guest.py > /results/controller.log 2>&1
echo FC_ARCHIVE_BEGIN
/bin/busybox tar -czf /tmp/results.tar.gz -C /results .
/bin/busybox base64 /tmp/results.tar.gz
echo FC_ARCHIVE_END
/bin/busybox poweroff -f
''')
    init.chmod(0o755)
    if args.protected_launcher:
        export = initroot / "lab/export-test.sh"
        export.write_text("#!/bin/sh\nexport PATH=" + init.read_text().split("export PATH=", 1)[1])
        init.write_text("#!/bin/sh\n/bin/busybox mount -t proc proc /proc\n/bin/busybox mount -t sysfs sysfs /sys\n/bin/busybox mount -t devtmpfs devtmpfs /dev\n/bin/busybox chmod 1777 /tmp\nexec /usr/lib/systemd/systemd --system --unit=fc-test.target --show-status=no\n")
    # cpio is given a NUL-delimited path list, with root ownership only INSIDE the archive.
    paths = b"\0".join(str(p.relative_to(initroot)).encode() for p in sorted(initroot.rglob("*"))) + b"\0"
    archive = subprocess.run(["cpio", "--null", "-o", "--format=newc", "--owner=0:0"],
                             input=paths, cwd=initroot, capture_output=True, check=True).stdout
    def device_entry(name, major, minor, inode):
        name = name.encode() + b"\0"
        fields = [inode, stat.S_IFCHR | 0o600, 0, 0, 1, 0, 0, 0, 0, major, minor, len(name), 0]
        entry = b"070701" + b"".join(f"{value:08x}".encode() for value in fields) + name
        return entry + b"\0" * (-len(entry) % 4)
    archive = device_entry("dev/console", 5, 1, 900001) + device_entry("dev/null", 1, 3, 900002) + archive
    initrd = out / "initramfs.cpio.gz"
    with gzip.open(initrd, "wb", compresslevel=1) as handle:
        handle.write(archive)
    cmd = [str(args.qemu.resolve()), "-L", str(args.qemu_share.resolve()),
           "-machine", "q35", "-accel", "tcg", "-cpu", "max", "-smp", "2", "-m", "1536",
           "-nodefaults", "-display", "none", "-serial", "stdio", "-monitor", "none", "-nic", "none",
           "-no-reboot", "-smbios", "type=1,product=FC proc visibility disposable VM",
           "-kernel", str(args.kernel.resolve()), "-initrd", str(initrd),
           "-append", "console=ttyS0 rdinit=/init panic=-1 random.trust_cpu=on"]
    if args.bios:
        cmd += ["-bios", str(args.bios.resolve())]
    metadata = {"command": cmd, "host_kernel": os.uname().release,
                "kernel_sha256": hashlib.sha256(args.kernel.read_bytes()).hexdigest(),
                "binary_sha256": {str(p.relative_to(initroot / "lab")): hashlib.sha256(p.read_bytes()).hexdigest()
                                  for p in (initroot / "lab").rglob("*") if p.is_file()},
                "qemu_version": subprocess.check_output([str(args.qemu.resolve()), "--version"], text=True).splitlines()[0],
                "host_proc_before": [line for line in Path("/proc/mounts").read_text().splitlines() if line.split()[1] == "/proc"]}
    print(f"Booting dedicated VM; evidence will be saved under {out}", flush=True)
    with (out / "serial.log").open("wb") as handle:
        try:
            result = subprocess.run(cmd, stdout=handle, stderr=subprocess.STDOUT, timeout=480)
            metadata["qemu_exit_code"] = result.returncode
        except subprocess.TimeoutExpired:
            metadata["timeout"] = True
    metadata["host_proc_after"] = [line for line in Path("/proc/mounts").read_text().splitlines() if line.split()[1] == "/proc"]
    (out / "run.json").write_text(json.dumps(metadata, indent=2) + "\n")
    serial = (out / "serial.log").read_text(errors="replace")
    match = re.search(r"FC_ARCHIVE_BEGIN\s+(.*?)\s+FC_ARCHIVE_END", serial, re.S)
    if not match:
        raise SystemExit(f"Guest did not return evidence; inspect {out / 'serial.log'}")
    data = base64.b64decode(match[1])
    (out / "guest-results.tar.gz").write_bytes(data)
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        archive.extractall(out / "results", filter="data")
    evidence = json.loads((out / "results/evidence.json").read_text())
    print(json.dumps({"pass": evidence["pass"], "measurements": len(evidence["measurements"]),
                      "host_proc_unchanged": metadata["host_proc_before"] == metadata["host_proc_after"],
                      "evidence": str(out / "results/evidence.json")}, indent=2))
    if not evidence["pass"]:
        print(evidence.get("error", ""))
        raise SystemExit(1)


if __name__ == "__main__":
    main()
