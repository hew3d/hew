"""The package recipes tell the app how it was installed.

shells/tauri/src-tauri/src/install_channel.rs reads `share/hew/install-channel`
next to the binary. Every repackaged .deb carries the Deb bundle marker, so
without this file a Flatpak, pacman, or Gentoo copy would hand an update to
`dpkg`. These tests run each recipe's real marker-writing step into a scratch
prefix; nothing is installed and no package tool runs.

Run with: python3 -m unittest discover -s packaging/tests -v
"""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MARKER = 'share/hew/install-channel'


def first_line(path: Path) -> str:
    return path.read_text().splitlines()[0]


class InstallChannelMarkerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.tmp = Path(self.temp.name)

    def test_flatpak_build_commands_write_the_flatpak_marker(self):
        manifest = (ROOT / 'packaging/flatpak/com.hew3d.Hew.yml').read_text()
        commands = [m.group(1) for m in re.finditer(r'^\s+- (.*install-channel.*)$', manifest, re.M)]
        self.assertEqual(len(commands), 1, commands)
        prefix = self.tmp / 'app'
        prefix.mkdir()
        subprocess.run(['sh', '-ec', commands[0].replace('/app', str(prefix))], check=True)
        self.assertEqual(first_line(prefix / MARKER), 'flatpak')

    def test_pacman_package_function_writes_the_pacman_marker(self):
        pkgdir = self.tmp / 'pkg'
        pkgdir.mkdir()
        # Run the recipe's own package() with the archive and chmod steps
        # stubbed out: only the marker is under test.
        script = (
            'bsdtar() { :; }; chmod() { :; }\n'
            f'source "{ROOT}/packaging/pacman/hew-bin/PKGBUILD"\n'
            f'pkgdir="{pkgdir}"\n'
            'package\n'
        )
        subprocess.run(['bash', '-eu', '-c', script], check=True, cwd=self.tmp)
        self.assertEqual(first_line(pkgdir / 'usr' / MARKER), 'pacman')

    def test_gentoo_ebuild_installs_the_gentoo_marker(self):
        # Portage is not available here; check the install step's shape and
        # that the validate script asserts the same value in the image.
        ebuild = next((ROOT / 'packaging/gentoo/media-gfx/hew-bin').glob('hew-bin-*.ebuild')).read_text()
        self.assertRegex(ebuild, r"printf 'gentoo\\n' > \"\$\{T\}\"/install-channel")
        self.assertRegex(ebuild, r'insinto /usr/share/hew\s+doins "\$\{T\}"/install-channel')
        validate = (ROOT / 'packaging/scripts/validate-gentoo-ebuild.sh').read_text()
        self.assertIn('usr/share/hew/install-channel', validate)
        self.assertIn('= "gentoo"', validate)

    def test_marker_values_are_names_the_app_recognizes(self):
        # install_channel.rs::parse_marker maps these three; keep the recipes
        # and the app in step.
        source = (ROOT / 'shells/tauri/src-tauri/src/install_channel.rs').read_text()
        for value in ('flatpak', 'pacman', 'gentoo'):
            self.assertIn(f'"{value}" => Some(InstallChannel::', source)


if __name__ == '__main__':
    unittest.main()
