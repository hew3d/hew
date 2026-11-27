"""packaging/scripts/smoke-flatpak-update.sh against `gh` and `flatpak` doubles.

The real run needs Flatpak and a published release, so these tests check the
script's decisions: which earlier release it updates from, which it skips,
that it retries while the new version is not served yet, and that it fails
loudly (with an ::error:: line) when the version never arrives.

Run with: python3 -m unittest discover -s packaging/tests -v
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'packaging/scripts/smoke-flatpak-update.sh'

DOUBLE = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
state_path = pathlib.Path(os.environ['SMOKE_STATE'])
state = json.loads(state_path.read_text())
state['calls'].append([name] + args)
def save(): state_path.write_text(json.dumps(state))
if name == 'gh':
    if args[:2] == ['release', 'list']:
        print('\n'.join(state['releases']))
    elif args[:2] == ['release', 'download']:
        if not state['bundle_available']:
            save(); sys.exit(1)
        out = pathlib.Path(args[args.index('--dir') + 1]) / 'hew.flatpak'
        out.write_text('bundle of ' + args[2])
        state['bundle_version'] = args[2].lstrip('v')
    save()
elif name == 'flatpak':
    cmd = args[0]
    if cmd == 'install' and any(a.endswith('hew.flatpak') for a in args):
        assert '--user' in args and '--no-deps' in args, args
        state['version'] = state['bundle_version']
    elif cmd == 'install':
        assert '--user' in args and 'hew' in args, args
        state['fresh_tries'] += 1
        if state['fresh_tries'] >= state['serves_after']:
            state['version'] = state['new']
    elif cmd == 'update':
        assert '--user' in args and '--no-deps' in args, args
        state['update_tries'] += 1
        if state['update_tries'] >= state['serves_after']:
            state['version'] = state['new']
    elif cmd == 'uninstall':
        state['version'] = None
    elif cmd == 'info':
        if state['version']:
            print('Hew - a 3D modeler\n          ID: com.hew3d.Hew\n     Version: ' + state['version'])
        else:
            save(); sys.exit(1)
    elif cmd == 'remote-add':
        pass
    else:
        raise AssertionError('unexpected flatpak command ' + cmd)
    save()
'''


class SmokeTests(unittest.TestCase):
    def run_smoke(self, releases, tag='v1.5.0', serves_after=1, bundle_available=True):
        with tempfile.TemporaryDirectory() as d:
            tmp = Path(d)
            bindir = tmp / 'bin'
            bindir.mkdir()
            for tool in ('gh', 'flatpak'):
                (bindir / tool).write_text(DOUBLE)
                (bindir / tool).chmod(0o755)
            state = tmp / 'state.json'
            state.write_text(json.dumps({
                'calls': [], 'releases': releases, 'version': None, 'new': tag.lstrip('v'),
                'bundle_version': None, 'bundle_available': bundle_available,
                'serves_after': serves_after, 'update_tries': 0, 'fresh_tries': 0}))
            env = dict(os.environ, PATH=f'{bindir}:{os.environ["PATH"]}', SMOKE_STATE=str(state),
                       SMOKE_ATTEMPTS='3', SMOKE_DELAY='0')
            result = subprocess.run(['bash', str(SCRIPT), tag], env=env, capture_output=True, text=True)
            return result, json.loads(state.read_text())

    @staticmethod
    def flatpak_calls(state):
        return [c[1] for c in state['calls'] if c[0] == 'flatpak']

    def test_updates_from_the_previous_release_then_installs_fresh(self):
        result, state = self.run_smoke(['v1.5.0', 'v1.4.0', 'v1.3.1'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(state['bundle_version'], '1.4.0')
        self.assertEqual(self.flatpak_calls(state).count('update'), 1)
        self.assertIn('install', self.flatpak_calls(state))
        self.assertEqual(state['version'], '1.5.0')

    def test_the_new_tag_need_not_be_listed_yet_and_the_previous_is_the_next_lower(self):
        result, state = self.run_smoke(['v1.4.0', 'v1.3.1', 'v1.4.1'], tag='v1.4.2')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(state['bundle_version'], '1.4.1')

    def test_retries_until_the_new_version_is_served(self):
        result, state = self.run_smoke(['v1.4.0'], serves_after=3)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(state['update_tries'], 3)
        self.assertIn('retrying', result.stdout)

    def test_fails_loudly_when_the_version_never_arrives(self):
        result, state = self.run_smoke(['v1.4.0'], serves_after=99)
        self.assertEqual(result.returncode, 1)
        self.assertIn('::error::', result.stderr)
        self.assertIn("wanted 1.5.0", result.stderr)

    def test_skips_a_previous_release_older_than_1_4_0(self):
        result, state = self.run_smoke(['v1.3.1', 'v1.3.0'])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('::notice::', result.stdout)
        self.assertEqual(self.flatpak_calls(state), [])

    def test_skips_when_there_is_no_earlier_release(self):
        result, state = self.run_smoke([], tag='v1.4.0')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.flatpak_calls(state), [])

    def test_a_previous_release_without_a_bundle_skips_only_the_update_half(self):
        result, state = self.run_smoke(['v1.4.0'], bundle_available=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn('update', self.flatpak_calls(state))
        self.assertEqual(state['version'], '1.5.0')  # the fresh install still ran


if __name__ == '__main__':
    unittest.main()
