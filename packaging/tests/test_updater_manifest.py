"""The updater manifest assembly and its guard, run against a fake release.

.github/scripts/build-updater-manifest.py is what the release workflow's
`updater-manifest` job runs. These tests feed it an asset list shaped like a
real release and check the JSON it writes, and that it fails loudly (nonzero
exit, `::error::` line) rather than ship a manifest with a missing key or a
blank signature.

Run with: python3 -m unittest discover -s packaging/tests -v
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / '.github/scripts/build-updater-manifest.py'
V = '1.5.0'
TAG = f'v{V}'
BASE = f'https://github.com/example/hew/releases/download/{TAG}/'

BUNDLES = [
    f'Hew_{V}_universal.app.tar.gz',
    f'Hew_{V}_x64-setup.exe',
    f'Hew_{V}_arm64-setup.exe',
    f'Hew_{V}_amd64.AppImage',
    f'Hew_{V}_aarch64.AppImage',
    f'Hew_{V}_amd64.deb',
    f'Hew_{V}_arm64.deb',
    f'Hew-{V}-1.x86_64.rpm',
    f'Hew-{V}-1.aarch64.rpm',
]
# Unsigned or unrelated assets a release also carries.
OTHER = [f'Hew_{V}_universal.dmg', f'hew-web-{TAG}.tar.gz', f'hew-relay-{TAG}-linux-x86_64.tar.gz']

CHANGELOG = f'''# Changelog

## [Unreleased]

### Fixed

- Something not released yet.

## [{V}] (2026-01-01) - Test

### Fixed

- Updates work on Linux.

## [1.4.0] (2025-12-01)

- Older.
'''


class UpdaterManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.tmp = Path(self.temp.name)
        (self.tmp / 'sigs').mkdir()
        (self.tmp / 'CHANGELOG.md').write_text(CHANGELOG)
        self.assets = list(BUNDLES + OTHER)
        for name in BUNDLES:
            self.sign(name)

    def sign(self, name, content=None):
        (self.tmp / 'sigs' / f'{name}.sig').write_text(content if content is not None else f'sig-of-{name}\n')
        if f'{name}.sig' not in self.assets:
            self.assets.append(f'{name}.sig')

    def run_script(self, tag=TAG, changelog=True):
        (self.tmp / 'assets.tsv').write_text(''.join(f'{n}\t{i}\n' for i, n in enumerate(self.assets)))
        cmd = [sys.executable, '-I', str(SCRIPT), '--repo', 'example/hew', '--tag', tag,
               '--assets', str(self.tmp / 'assets.tsv'), '--sigs', str(self.tmp / 'sigs'),
               '--out', str(self.tmp / 'latest.json'), '--pub-date', '2026-01-02T03:04:05Z']
        if changelog:
            cmd += ['--changelog', str(self.tmp / 'CHANGELOG.md')]
        return subprocess.run(cmd, capture_output=True, text=True)

    def manifest(self):
        return json.loads((self.tmp / 'latest.json').read_text())

    def test_every_bundle_type_has_an_explicit_key_and_the_bare_keys_stay_appimage(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        platforms = self.manifest()['platforms']
        expect = {
            'darwin-x86_64': f'Hew_{V}_universal.app.tar.gz',
            'darwin-aarch64': f'Hew_{V}_universal.app.tar.gz',
            'windows-x86_64': f'Hew_{V}_x64-setup.exe',
            'windows-aarch64': f'Hew_{V}_arm64-setup.exe',
            'linux-x86_64': f'Hew_{V}_amd64.AppImage',
            'linux-aarch64': f'Hew_{V}_aarch64.AppImage',
            'linux-x86_64-appimage': f'Hew_{V}_amd64.AppImage',
            'linux-aarch64-appimage': f'Hew_{V}_aarch64.AppImage',
            'linux-x86_64-deb': f'Hew_{V}_amd64.deb',
            'linux-aarch64-deb': f'Hew_{V}_arm64.deb',
            'linux-x86_64-rpm': f'Hew-{V}-1.x86_64.rpm',
            'linux-aarch64-rpm': f'Hew-{V}-1.aarch64.rpm',
        }
        self.assertEqual(sorted(platforms), sorted(expect))
        for key, asset in expect.items():
            self.assertEqual(platforms[key], {'url': BASE + asset, 'signature': f'sig-of-{asset}'}, key)

    def test_version_date_and_notes(self):
        self.assertEqual(self.run_script().returncode, 0)
        m = self.manifest()
        self.assertEqual(m['version'], V)
        self.assertEqual(m['pub_date'], '2026-01-02T03:04:05Z')
        self.assertEqual(m['notes'], '### Fixed\n\n- Updates work on Linux.')

    def test_notes_fall_back_with_a_warning_without_a_changelog_section(self):
        # A pre-release tag has no section of its own, and the manifest
        # carries its own tag's version and URLs.
        result = self.run_script(tag='v1.6.0-pre.x')
        self.assertEqual(result.returncode, 0, result.stderr)
        m = self.manifest()
        self.assertEqual((m['version'], m['notes']), ('1.6.0-pre.x', 'Hew 1.6.0-pre.x'))
        self.assertIn('::warning::', result.stderr)
        self.assertTrue(m['platforms']['linux-x86_64-deb']['url'].startswith(
            'https://github.com/example/hew/releases/download/v1.6.0-pre.x/'))
        # No changelog file at all.
        self.assertEqual(self.run_script(changelog=False).returncode, 0)
        self.assertEqual(self.manifest()['notes'], f'Hew {V}')
        # A changelog with no section for this version.
        (self.tmp / 'CHANGELOG.md').write_text('# Changelog\n\n## [0.1.0]\n\n- Old.\n')
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.manifest()['notes'], f'Hew {V}')

    def test_missing_bundle_fails_loudly(self):
        for victim in (f'Hew_{V}_amd64.deb', f'Hew-{V}-1.aarch64.rpm', f'Hew_{V}_amd64.AppImage'):
            with self.subTest(victim=victim):
                self.setUp()
                self.assets.remove(victim)
                result = self.run_script()
                self.assertEqual(result.returncode, 1)
                self.assertIn('::error::', result.stderr)
                self.assertFalse((self.tmp / 'latest.json').exists())

    def test_missing_or_empty_signature_fails_loudly(self):
        for victim in (f'Hew_{V}_arm64.deb', f'Hew-{V}-1.x86_64.rpm'):
            with self.subTest(victim=victim, case='not on the release'):
                self.setUp()
                self.assets.remove(f'{victim}.sig')
                result = self.run_script()
                self.assertEqual(result.returncode, 1)
                self.assertIn(f'Missing {victim}.sig', result.stderr)
            with self.subTest(victim=victim, case='not downloaded'):
                self.setUp()
                (self.tmp / 'sigs' / f'{victim}.sig').unlink()
                self.assertEqual(self.run_script().returncode, 1)
            with self.subTest(victim=victim, case='blank'):
                self.setUp()
                self.sign(victim, content='\n')
                result = self.run_script()
                self.assertEqual(result.returncode, 1)
                self.assertIn('is empty', result.stderr)

    def test_a_signed_bundle_with_no_key_fails_the_guard(self):
        # A new kind of desktop bundle appears, signed, but the assembly has
        # no key for it: the job must fail rather than leave it unreachable.
        for extra in (f'Hew_{V}_amd64.msi', f'Hew_{V}_riscv64.deb'):
            with self.subTest(extra=extra):
                self.setUp()
                self.assets.append(extra)
                self.sign(extra)
                result = self.run_script()
                # A second .deb for an arch with no key either trips the
                # exactly-one rule (not for riscv64) or the guard.
                self.assertEqual(result.returncode, 1)
                self.assertIn(extra, result.stderr)

    def test_signatures_on_non_desktop_assets_do_not_trip_the_guard(self):
        self.assets.append(f'hew-web-{TAG}.tar.gz.sig')
        (self.tmp / 'sigs' / f'hew-web-{TAG}.tar.gz.sig').write_text('x')
        self.assertEqual(self.run_script().returncode, 0)

    def test_two_assets_for_one_key_fails(self):
        extra = f'Hew_{V}_amd64-extra.deb'
        self.assets.append(extra)
        self.sign(extra)
        result = self.run_script()
        self.assertEqual(result.returncode, 1)
        self.assertIn('Expected exactly one asset', result.stderr)

    def test_the_manifest_the_old_clients_read_is_still_valid(self):
        # Clients before the explicit keys read only the bare linux-<arch>
        # keys; those must still point at the AppImage.
        self.assertEqual(self.run_script().returncode, 0)
        p = self.manifest()['platforms']
        self.assertTrue(p['linux-x86_64']['url'].endswith('.AppImage'))
        self.assertTrue(p['linux-aarch64']['url'].endswith('.AppImage'))


if __name__ == '__main__':
    unittest.main()
