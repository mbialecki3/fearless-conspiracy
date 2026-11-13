#!/usr/bin/env python3
"""Policy tests that never change host mounts, accounts, or cgroups."""
import importlib.util
from contextlib import contextmanager
import json
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

    @contextmanager
    def provisioning(self):
        accounts, calls = {}, []
        original_lstat = Path.lstat

        def root_owned(path):
            info = original_lstat(path)
            return SimpleNamespace(st_uid=0, st_mode=info.st_mode)

        def find(name):
            if name not in accounts:
                raise KeyError(name)
            return accounts[name]

        def execute(program, *args, input=None):
            calls.append((program, args, input))
            if program == "useradd" and "--help" not in args:
                name = args[-1]
                uid = 2000 + len(accounts)
                accounts[name] = pwd.struct_passwd((name, "x", uid, uid, args[args.index("--comment")+1],
                                                    args[args.index("--home-dir")+1], args[args.index("--shell")+1]))
            if program == "userdel" and "--help" not in args:
                del accounts[args[-1]]
            return ""

        with tempfile.TemporaryDirectory() as temporary, patch.object(Path, "lstat", root_owned), \
             patch.object(launcher, "STATE", Path(temporary) / "state"), \
             patch.object(launcher, "HOMES", Path(temporary) / "home"), \
             patch.object(launcher.pwd, "getpwnam", side_effect=find), \
             patch.object(launcher.grp, "getgrnam", side_effect=KeyError), \
             patch.object(launcher, "command", side_effect=execute), patch("builtins.print"):
            yield accounts, calls

    def test_provisioning_creates_private_accounts_and_keeps_passwords_on_reuse(self):
        with self.provisioning() as (accounts, calls):
            server, players = launcher.provision_accounts("fearless_host", 3)
            self.assertEqual(server.pw_shell, "/usr/sbin/nologin")
            self.assertEqual(len({p.pw_uid for p in [server, *players]}), 4)
            registry = launcher.STATE / "accounts.json"
            before = json.loads(registry.read_text())
            passwords = [before[p.pw_name]["password"] for p in players]
            self.assertEqual(len(set(passwords)), 3)
            self.assertTrue(all(len(password) >= 20 for password in passwords))
            self.assertEqual(registry.stat().st_mode & 0o777, 0o600)
            self.assertEqual((launcher.STATE / "player-logins.txt").stat().st_mode & 0o777, 0o600)
            launcher.provision_accounts("fearless_host", 3)
            after = json.loads(registry.read_text())
            self.assertEqual(before, after)
            self.assertEqual(sum(program == "chpasswd" and "--help" not in args for program, args, _ in calls), 3)
            self.assertTrue(all(password not in str(args) for _, args, _ in calls for password in passwords))
            launcher.provision_accounts("fearless_host", 5)
            self.assertEqual(len(accounts), 6)
            self.assertEqual(before["fearless_p1"], json.loads(registry.read_text())["fearless_p1"])

    def test_provisioning_refuses_to_adopt_an_unrelated_account(self):
        with self.provisioning() as (accounts, calls):
            accounts["fearless_p3"] = entry("fearless_p3", 3000, 3000)
            with self.assertRaisesRegex(RuntimeError, "not managed"):
                launcher.provision_accounts("fearless_host", 3)
            self.assertFalse(any(program == "useradd" and "--help" not in args for program, args, _ in calls))

    def test_provisioning_rejects_replaced_account_identities(self):
        with self.provisioning() as (accounts, calls):
            launcher.provision_accounts("fearless_host", 3)
            accounts["fearless_p1"] = entry("fearless_p1", 4000, 4000)
            count = len(calls)
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                launcher.provision_accounts("fearless_host", 3)
            self.assertFalse(any(program == "chpasswd" and "--help" not in args for program, args, _ in calls[count:]))

    def test_provisioning_recovers_an_interrupted_password_setup(self):
        with self.provisioning() as (accounts, calls):
            original = launcher.command
            attempted = []

            def interrupted(program, *args, input=None):
                if program == "chpasswd" and input and input.startswith("fearless_p2:"):
                    attempted.append(input)
                    raise subprocess.CalledProcessError(1, [program])
                return original(program, *args, input=input)

            with patch.object(launcher, "command", side_effect=interrupted):
                with self.assertRaises(subprocess.CalledProcessError):
                    launcher.provision_accounts("fearless_host", 3)
            launcher.provision_accounts("fearless_host", 3)
            registry = json.loads((launcher.STATE / "accounts.json").read_text())
            self.assertTrue(all(record["ready"] for record in registry.values()))
            self.assertEqual(attempted, [f"fearless_p2:{registry['fearless_p2']['password']}\n"])
            self.assertEqual(sum(program == "useradd" and "--help" not in args for program, args, _ in calls), 4)

    def test_cleanup_removes_only_managed_players_and_their_credentials(self):
        with self.provisioning() as (accounts, calls):
            launcher.provision_accounts("fearless_host", 3)
            accounts["outsider"] = entry("outsider", 5000, 5000)
            errors = launcher.cleanup_players(["fearless_host", "outsider", "fearless_p1", "fearless_p2", "fearless_p3"])
            self.assertEqual(errors, [])
            self.assertEqual(set(accounts), {"fearless_host", "outsider"})
            self.assertEqual(set(json.loads((launcher.STATE / "accounts.json").read_text())), {"fearless_host"})
            self.assertFalse((launcher.STATE / "player-logins.txt").exists())
            self.assertTrue(all("--force" not in args for _, args, _ in calls))

    def test_cleanup_refuses_replaced_identities(self):
        with self.provisioning() as (accounts, calls):
            launcher.provision_accounts("fearless_host", 3)
            accounts["fearless_p1"] = entry("fearless_p1", 4000, 4000)
            errors = launcher.cleanup_players(["fearless_p1"])
            self.assertTrue(errors)
            self.assertIn("identity changed", errors[0])
            self.assertFalse(any(program == "userdel" for program, _, _ in calls))

    def test_cleanup_failure_is_recorded_for_retry(self):
        with self.provisioning() as (accounts, calls):
            launcher.provision_accounts("fearless_host", 3)
            original = launcher.command

            def blocked(program, *args, input=None):
                if program == "userdel":
                    raise subprocess.CalledProcessError(8, [program])
                return original(program, *args, input=input)

            with patch.object(launcher, "command", side_effect=blocked):
                self.assertTrue(launcher.cleanup_players(["fearless_p1"]))
            record = json.loads((launcher.STATE / "accounts.json").read_text())["fearless_p1"]
            self.assertTrue(record["deleting"])
            self.assertEqual(launcher.cleanup_players(["fearless_p1"]), [])

    def test_only_one_protected_game_can_own_player_accounts(self):
        with self.provisioning():
            with launcher.game_session([]):
                with self.assertRaisesRegex(RuntimeError, "already running"):
                    with launcher.game_session([]):
                        self.fail("overlapping session was accepted")

    def test_failed_startup_cleans_partially_provisioned_players(self):
        with self.provisioning() as (accounts, calls):
            original = launcher.command

            def blocked(program, *args, input=None):
                if program == "chpasswd" and input and input.startswith("fearless_p2:"):
                    raise subprocess.CalledProcessError(1, [program])
                return original(program, *args, input=input)

            claimed = []
            with self.assertRaises(subprocess.CalledProcessError):
                with launcher.game_session(claimed), patch.object(launcher, "command", side_effect=blocked):
                    launcher.provision_accounts("fearless_host", 3, cleanup=claimed)
            self.assertEqual(set(accounts), {"fearless_host"})

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


class IsolationRestorationTests(unittest.TestCase):
    @contextmanager
    def fixture(self, active=True):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state, overrides, cgroup = root / "state", root / "overrides", root / "cgroup"
            state.mkdir(mode=0o700)
            entry_ = entry("player", 2001, 2001)
            directory = cgroup / "user.slice/user-2001.slice"
            directory.mkdir(parents=True)
            original_values = {"cpu.max": "60000 100000", "memory.high": "314572800",
                               "memory.max": "419430400", "memory.swap.max": "67108864",
                               "pids.max": "90", "cgroup.max.descendants": "77"}
            for name, value in original_values.items():
                (directory / name).write_text(value)
            stat_ = Path.stat
            lstat_ = Path.lstat

            def owned(info):
                return SimpleNamespace(st_uid=0, st_gid=info.st_gid, st_mode=info.st_mode)

            def execute(program, *args, input=None):
                if program == "systemctl" and "ActiveState" in str(args):
                    return "active" if active else "inactive"
                if program == "getfacl":
                    return "user::rwx\ngroup::r-x\nother::--x"
                return ""

            record = {"slices": [], "proc": None}
            with patch.object(launcher, "STATE", state), patch.object(launcher, "OVERRIDES", overrides), \
                 patch.object(launcher, "CGROUP", cgroup), patch.object(launcher.os, "chown") as chown, \
                 patch.object(Path, "stat", lambda p, **kw: owned(stat_(p, **kw))), \
                 patch.object(Path, "lstat", lambda p: owned(lstat_(p))), \
                 patch.object(launcher, "command", side_effect=execute) as command_:
                yield record, entry_, directory, overrides / "user-2001.slice.d", state, command_, chown

    def test_existing_overrides_limits_acl_and_unrelated_files_are_preserved(self):
        with self.fixture() as (record, player, directory, dropins, state, command_, chown):
            dropins.mkdir(mode=0o755, parents=True)
            original = "# administrator settings\n[Slice]\nCPUQuota=60%\n"
            (dropins / "50-CPUQuota.conf").write_text(original)
            (dropins / "50-CPUQuota.conf").chmod(0o644)
            (dropins / "50-CPUWeight.conf").write_text("[Slice]\nCPUWeight=42\n")
            launcher.remember_slice(record, player)
            for key in launcher.PLAYER_LIMITS:
                (dropins / f"50-{key}.conf").write_text("game setting")
                (dropins / f"50-{key}.conf").chmod(0o644)
            for path in directory.iterdir():
                path.write_text("changed")
            self.assertEqual(launcher.restore_isolation(record), [])
            self.assertEqual((dropins / "50-CPUQuota.conf").read_text(), original)
            self.assertEqual((dropins / "50-CPUQuota.conf").stat().st_mode & 0o777, 0o644)
            self.assertEqual((dropins / "50-CPUWeight.conf").read_text(), "[Slice]\nCPUWeight=42\n")
            self.assertFalse((dropins / "50-MemoryMax.conf").exists())
            self.assertEqual((directory / "cpu.max").read_text(), "60000 100000")
            self.assertEqual((directory / "cgroup.max.descendants").read_text(), "77")
            self.assertTrue(any(call.args[:2] == ("setfacl", "--set-file=-") for call in command_.call_args_list))
            self.assertTrue(chown.called)
            self.assertFalse(any("revert" in call.args for call in command_.call_args_list))

    def test_partial_setup_removes_only_new_overrides_and_directory(self):
        with self.fixture(active=False) as (record, player, directory, dropins, state, command_, chown):
            launcher.remember_slice(record, player)
            dropins.mkdir(mode=0o755, parents=True)
            (dropins / "50-CPUQuota.conf").write_text("partially applied")
            (dropins / "50-CPUQuota.conf").chmod(0o644)
            self.assertEqual(launcher.restore_isolation(record), [])
            self.assertFalse(dropins.exists())
            self.assertTrue(any(call.args == ("systemctl", "show", "--property=ActiveState", "--value", "user-2001.slice") for call in command_.call_args_list))

    def test_symlink_override_is_rejected_before_modification(self):
        with self.fixture() as (record, player, directory, dropins, state, command_, chown):
            dropins.mkdir(mode=0o755, parents=True)
            target = state / "unrelated"
            target.write_text("unchanged")
            (dropins / "50-MemoryMax.conf").symlink_to(target)
            with self.assertRaisesRegex(RuntimeError, "unsafe runtime override"):
                launcher.remember_slice(record, player)
            self.assertEqual(target.read_text(), "unchanged")
            self.assertEqual(record["slices"], [])

    def test_no_relaxation_if_stopping_the_game_fails(self):
        session = {"unit": "game.scope", "process": None, "isolation": {}}
        with patch.object(launcher, "stop_unit", side_effect=RuntimeError("stop failed")), \
             patch.object(launcher, "restore_isolation") as restore, patch.object(launcher, "cleanup_players") as cleanup:
            self.assertIn("isolation retained", launcher.finish_session(session, ["p1"])[0])
            restore.assert_not_called()
            cleanup.assert_not_called()

    def test_restoration_failure_keeps_recovery_record_for_retry(self):
        with self.fixture() as (record, player, directory, dropins, state, command_, chown):
            journal = state / "isolation.json"
            journal.write_text("saved state")
            session = {"unit": None, "process": None, "socket": None, "isolation": record}
            with patch.object(launcher, "restore_isolation", return_value=["restore failed"]):
                self.assertEqual(launcher.finish_session(session, []), ["restore failed"])
            self.assertTrue(journal.exists())
            with patch.object(launcher, "restore_isolation", return_value=[]), patch("builtins.print"):
                self.assertEqual(launcher.finish_session(session, []), [])
            self.assertFalse(journal.exists())

    def test_proc_is_kept_restricted_when_resource_restoration_fails(self):
        with self.fixture() as (record, player, directory, dropins, state, command_, chown):
            launcher.remember_slice(record, player)
            record["proc"] = {"id": "1", "flags": 32, "options": "hidepid=0,gid=0"}
            with patch.object(launcher, "command", side_effect=RuntimeError("systemd unavailable")), \
                 patch.object(launcher, "remount_proc") as remount:
                self.assertTrue(launcher.restore_isolation(record))
                remount.assert_not_called()

    def test_proc_snapshot_normalizes_aliases_and_preserves_other_options(self):
        info = "1 0 0:1 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw,gid=2999,hidepid=noaccess,subset=pid\n"
        with patch.object(Path, "read_text", return_value=info):
            saved = launcher.proc_mount()
        self.assertEqual(saved["options"], "subset=pid,hidepid=1,gid=2999")
        libc = Mock()
        libc.mount.return_value = 0
        with patch.object(launcher.ctypes, "CDLL", return_value=libc):
            launcher.remount_proc(saved, saved["options"], "cannot restore")
        self.assertEqual(libc.mount.call_args.args[-1], b"subset=pid,hidepid=1,gid=2999")

    def test_pending_recovery_completes_before_new_session(self):
        with self.fixture() as (record, player, directory, dropins, state, command_, chown):
            pending = {"version": 1, "boot_id": "test-boot", "unit": "old.scope", "socket": "game.sock",
                       "server_uid": 2000, "players": ["p1"], "slices": [], "proc": None}
            launcher.save_isolation(pending)
            calls = []
            def finish(session, players):
                calls.append((session["unit"], list(players)))
                return []
            with patch.object(launcher, "boot_id", return_value="test-boot"), \
                 patch.object(launcher, "finish_session", side_effect=finish):
                with launcher.game_session([]):
                    self.assertEqual(calls, [("old.scope", ["p1"])])
            self.assertFalse((state / "isolation.json").exists())


if __name__ == "__main__":
    unittest.main()
