"""Exercise packaging commands without downloading runtimes or using signing secrets.

Run with: python3 -m unittest discover -s packaging/tests -v
"""
import base64
import configparser
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]

# The external tools are unavailable on macOS. This double models their CLI
# boundary: bundle metadata and separate per-user/system remote installations.
TOOL = r'''#!/usr/bin/env python3
import base64, json, os, pathlib, sys
args = sys.argv[1:]
name = pathlib.Path(sys.argv[0]).name
if name == 'gpg':
    if '--list-secret-keys' in args: print('sec:-:4096:1:TESTKEY:')
    elif '--export' in args: sys.stdout.buffer.write(b'test-public-key')
    else: sys.stdin.read()
elif name == 'base64':
    print(base64.b64encode(sys.stdin.buffer.read()).decode(), end='')
elif name == 'flatpak-builder':
    assert '--gpg-sign=TESTKEY' in args
    pathlib.Path(args[-2]).mkdir(exist_ok=True)
elif name == 'flatpak':
    command = args.pop(0)
    flags = {a.split('=', 1)[0]: a.split('=', 1)[1] if '=' in a else True
             for a in args if a.startswith('--')}
    positional = [a for a in args if not a.startswith('--')]
    if command == 'build-update-repo':
        assert flags['--gpg-sign'] == 'TESTKEY'
    elif command == 'build-bundle':
        assert flags['--gpg-sign'] == 'TESTKEY'
        key = flags.get('--gpg-keys')
        pathlib.Path(positional[1]).write_text(json.dumps({
            'origin': flags.get('--repo-url'),
            'runtime-repo': flags.get('--runtime-repo'),
            'gpg-keys': pathlib.Path(key).read_text() if key else None,
            'app': positional[2],
        }))
    else:
        state_path = pathlib.Path(os.environ['FLATPAK_STATE'])
        state = json.loads(state_path.read_text())
        scope = 'user' if '--user' in flags else 'system'
        if command == 'remote-add':
            remote, url = positional
            state[scope]['remotes'][remote] = url
        elif command == 'install':
            remote, app = positional
            assert remote in state[scope]['remotes'], 'app remote missing in ' + scope
            assert 'flathub' in state[scope]['remotes'], 'GNOME runtime missing in ' + scope
            state[scope]['installed'].append(app)
        else: raise AssertionError('unexpected command: ' + command)
        state_path.write_text(json.dumps(state))
else: raise AssertionError('unexpected tool: ' + name)
'''


class FlatpakPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.tmp = Path(self.temp.name)
        tools = self.tmp / 'bin'
        tools.mkdir()
        for name in ('gpg', 'base64', 'flatpak-builder', 'flatpak'):
            script = tools / name
            script.write_text(TOOL)
            script.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{tools}:{os.environ["PATH"]}',
                        PACKAGING_GPG_PRIVATE_KEY='test-only-private-key',
                        FLATPAK_STATE=str(self.tmp / 'state.json'))

    def test_bundle_has_runtime_source_update_origin_and_verification_key(self):
        for suffix in ('', '/'):
            with self.subTest(base_url_suffix=suffix):
                repo = self.tmp / 'output' / 'repo'
                bundle = self.tmp / 'release' / 'hew.flatpak'
                subprocess.run(['bash', str(ROOT / 'packaging/scripts/build-flatpak-repo.sh'),
                                str(repo), str(bundle), 'https://example.test/packages' + suffix],
                               env=self.env, check=True, capture_output=True, text=True)
                metadata = json.loads(bundle.read_text())
                self.assertEqual(metadata['runtime-repo'],
                                 'https://dl.flathub.org/repo/flathub.flatpakrepo')
                self.assertEqual(metadata['origin'], 'https://example.test/packages/flatpak/repo')
                self.assertEqual(metadata['gpg-keys'], 'test-public-key')
                self.assertEqual(metadata['app'], 'com.hew3d.Hew')
                config = configparser.ConfigParser()
                config.read(repo.parent / 'com.hew3d.Hew.flatpakrepo')
                self.assertEqual(config['Flatpak Repo']['Url'], metadata['origin'])
                self.assertEqual(base64.b64decode(config['Flatpak Repo']['GPGKey']),
                                 b'test-public-key')

    def test_published_install_commands_work_with_user_only_or_missing_flathub(self):
        html = (ROOT / 'packaging/pages/index.html').read_text()
        commands = next(block for block in re.findall(r'<pre><code>(.*?)</code></pre>', html, re.S)
                        if block.startswith('flatpak '))
        for existing_flathub in (False, True):
            with self.subTest(existing_user_flathub=existing_flathub):
                state = {scope: {'remotes': {}, 'installed': []} for scope in ('user', 'system')}
                if existing_flathub:
                    state['user']['remotes']['flathub'] = 'https://dl.flathub.org/repo/flathub.flatpakrepo'
                state_path = Path(self.env['FLATPAK_STATE'])
                state_path.write_text(json.dumps(state))
                result = subprocess.run(['bash', '-eu', '-c', commands], env=self.env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                installed = json.loads(state_path.read_text())
                self.assertEqual(installed['user']['installed'], ['com.hew3d.Hew'])
                self.assertEqual(installed['system'], state['system'])
