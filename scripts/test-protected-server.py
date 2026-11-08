#!/usr/bin/env python3
"""Policy tests that never change host mounts, accounts, or cgroups."""
import importlib.util
from pathlib import Path
import pwd
import subprocess
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("protected_server", Path(__file__).with_name("protected-server.py"))
launcher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launcher)


def entry(name, uid, gid):
    return pwd.struct_passwd((name, "x", uid, gid, "", "/tmp", "/bin/sh"))


class ProtectedStartupTests(unittest.TestCase):
    def setUp(self):
        self.server = entry("host", 2000, 2000)
        self.players = [entry(f"p{i}", 2000+i, 2000+i) for i in range(1, 4)]

    def test_account_aliases_and_shared_groups_are_rejected(self):
        for players in ([self.players[0]] * 3,
                        [self.server, *self.players[:2]],
                        [entry("p1", 2001, 2000), *self.players[1:]]):
            with self.assertRaises(RuntimeError):
                launcher.validate_accounts(self.server, players)

    def test_sudo_variants_must_explicitly_deny_privileges(self):
        denial = "User p1 is not allowed to run sudo on localhost.\n"
        for code in (0, 1):
            result = subprocess.CompletedProcess([], code, denial, "")
            self.assertTrue(launcher.sudo_denies(result, "p1"))
        for result in [
            subprocess.CompletedProcess([], 0, "User p1 may run the following commands:\n(root) ALL", ""),
            subprocess.CompletedProcess([], 1, "", "policy plugin failed"),
            subprocess.CompletedProcess([], 0, denial, "sudoers syntax error"),
            subprocess.CompletedProcess([], 0, denial.replace("p1", "p2"), ""),
        ]:
            self.assertFalse(launcher.sudo_denies(result, "p1"))

    def test_membership_in_another_players_private_group_is_rejected(self):
        with patch.object(launcher, "groups", return_value={2001, 2002}):
            with self.assertRaisesRegex(RuntimeError, "another game account"):
                launcher.validate_accounts(self.server, self.players)

    def test_server_account_must_not_have_administrative_group_access(self):
        membership = lambda entry: {entry.pw_gid} | ({27} if entry.pw_uid == 2000 else set())
        with patch.object(launcher, "groups", side_effect=membership), patch.object(launcher.grp, "getgrgid", side_effect=lambda gid: SimpleNamespace(gr_name="sudo" if gid == 27 else f"group{gid}")), patch.object(Path, "exists", return_value=False):
            with self.assertRaisesRegex(RuntimeError, "privileged group"):
                launcher.validate_accounts(self.server, self.players)

    def test_remount_keeps_security_flags_and_removes_group_exemption(self):
        libc = Mock()
        libc.mount.return_value = 0
        mountinfo = "1 0 0:1 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw,hidepid=0,gid=999\n"
        with patch.object(Path, "read_text", return_value=mountinfo), patch.object(launcher.ctypes, "CDLL", return_value=libc):
            launcher.restrict_proc()
        self.assertEqual(libc.mount.call_args.args, (None, b"/proc", None, 32 | 2 | 4 | 8 | (1 << 21), b"hidepid=2,gid=0"))

    def test_a_failed_remount_aborts(self):
        libc = Mock()
        libc.mount.return_value = -1
        with patch.object(Path, "read_text", return_value="1 0 0:1 / /proc rw - proc proc rw\n"), patch.object(launcher.ctypes, "CDLL", return_value=libc):
            with self.assertRaisesRegex(RuntimeError, "cannot restrict"):
                launcher.restrict_proc()

    def test_visibility_probe_detects_readable_process_metadata(self):
        result = subprocess.run(["/usr/bin/python3", "-I", "-c", launcher.PROBE, "self", "/tmp"],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("private metadata remains readable", result.stderr)

    def test_unprivileged_launch_clears_all_capabilities(self):
        with patch.object(launcher, "groups", return_value={2000}):
            args = launcher.unprivileged(self.server, [2001, 2002])
        self.assertIn("--no-new-privs", args)
        self.assertIn("--bounding-set=-all", args)
        self.assertIn("--inh-caps=-all", args)
        self.assertIn("--ambient-caps=-all", args)
        self.assertIn("2000,2001,2002", args)

    def test_staging_replaces_a_symlink_without_overwriting_its_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "built-server"
            source.write_bytes(b"\x7fELFtest")
            victim = root / "private-file"
            victim.write_text("unchanged")
            (root / "fearless-conspiracy-server").symlink_to(victim)
            destination = launcher.stage_binary(source, root)
            self.assertFalse(destination.is_symlink())
            self.assertEqual(destination.read_bytes(), source.read_bytes())
            self.assertEqual(destination.stat().st_mode & 0o7777, 0o555)
            self.assertEqual(victim.read_text(), "unchanged")


if __name__ == "__main__":
    unittest.main()
