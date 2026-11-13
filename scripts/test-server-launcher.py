#!/usr/bin/env python3
"""Check wrapper routing in a disposable fixture; never change OS isolation."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ServerLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='fc-launcher-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copy2(ROOT / 'server', self.root / 'server')
        (self.root / 'scripts').mkdir()
        (self.root / 'bin').mkdir()
        stub = ('#!/usr/bin/python3\nimport json,sys\n'
                'print(json.dumps({"mode": MODE, "args": sys.argv[1:]}))\n')
        (self.root / 'scripts/protected-server.py').write_text(stub.replace('MODE', '"protected"'))
        binary = self.root / 'bin/fearless-conspiracy-server'
        binary.write_text(stub.replace('MODE', '"unprotected"'))
        binary.chmod(0o755)

    def launch(self, *args):
        env = dict(os.environ)
        env.pop('FEARLESS_CONSPIRACY_SOCKET', None)
        return subprocess.run(['bash', str(self.root / 'server'), *args],
                              capture_output=True, text=True, env=env, timeout=10)

    def decoded(self, *args):
        result = self.launch(*args)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_no_arguments_select_protection(self):
        self.assertEqual(self.decoded(), {'mode': 'protected', 'args': []})

    def test_player_count_does_not_disable_protection(self):
        self.assertEqual(self.decoded('--players', '5'),
                         {'mode': 'protected', 'args': ['--players', '5']})

    def test_failed_protected_startup_never_falls_back(self):
        (self.root / 'scripts/protected-server.py').write_text('raise SystemExit(7)\n')
        result = self.launch('--players', '5')
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, '')

    def test_explicit_protected_remains_compatible(self):
        self.assertEqual(self.decoded('--players', '5', '--protected')['args'], ['--players', '5'])

    def test_game_options_remain_after_separator(self):
        self.assertEqual(self.decoded('--players', '5', '--', '--manual', '--scenario')['args'],
                         ['--players', '5', '--', '--manual', '--scenario'])

    def test_mode_after_separator_cannot_disable_protection(self):
        result = self.decoded('--', '--unprotected')
        self.assertEqual(result, {'mode': 'protected', 'args': ['--', '--unprotected']})

    def test_option_value_cannot_select_a_mode(self):
        result = self.decoded('--socket', '--unprotected')
        self.assertEqual(result, {'mode': 'protected', 'args': ['--socket', '--unprotected']})

    def test_modes_cannot_be_combined_in_either_order(self):
        for args in [('--protected', '--unprotected'), ('--unprotected', '--protected')]:
            result = self.launch(*args)
            self.assertEqual(result.returncode, 2)
            self.assertIn('Cannot combine', result.stderr)
            self.assertEqual(result.stdout, '')

    def test_help_does_not_start_any_server(self):
        for args in [('--help',), ('--unprotected', '--help')]:
            result = self.launch(*args)
            self.assertEqual(result.returncode, 0)
            self.assertIn('Protected startup is the default', result.stdout)
            self.assertNotIn('"mode":', result.stdout)

    def test_missing_option_value_is_rejected(self):
        result = self.launch('--players')
        self.assertEqual(result.returncode, 2)
        self.assertIn('requires a value', result.stderr)

    @unittest.skipIf(os.geteuid() == 0, 'unprotected mode intentionally refuses root')
    def test_unprotected_requires_explicit_opt_out(self):
        result = self.decoded('--unprotected', '--players', '5')
        self.assertEqual(result, {'mode': 'unprotected',
            'args': ['/tmp/fearless-conspiracy.sock', '--setup', '--players', '5', '--allow-same-uid']})

    @unittest.skipIf(os.geteuid() == 0, 'unprotected mode intentionally refuses root')
    def test_unprotected_flag_can_follow_other_options(self):
        self.assertEqual(self.decoded('--players', '5', '--unprotected')['mode'], 'unprotected')

    @unittest.skipIf(os.geteuid() == 0, 'unprotected mode intentionally refuses root')
    def test_unprotected_strict_users_and_custom_socket(self):
        result = self.decoded('--unprotected', '--strict-users', '--shared-local', '--socket', '/tmp/test.sock')
        self.assertEqual(result, {'mode': 'unprotected',
            'args': ['/tmp/test.sock', '--setup', '--shared-local']})


if __name__ == '__main__':
    unittest.main()
