#!/usr/bin/env python3
"""Assemble the updater's latest.json from a release's asset list.

Run by the `updater-manifest` job in .github/workflows/release.yml, and by
packaging/tests/test_updater_manifest.py with a fake asset list, so the
assembly and its guard are testable without a release.

    build-updater-manifest.py --repo OWNER/REPO --tag vX.Y.Z \
        --assets assets.tsv --sigs SIG_DIR [--changelog CHANGELOG.md] \
        [--out latest.json] [--pub-date 2026-01-01T00:00:00Z]

`assets.tsv` has one release asset per line; the first tab-separated column
is the asset name (the release job's `name<TAB>id` listing works as is).
`SIG_DIR` holds each `.sig` asset's contents under its asset name.

The manifest keys follow tauri-plugin-updater's lookup: it asks for
`<os>-<arch>-<installer>` first and falls back to `<os>-<arch>`. On Linux the
installer is `appimage`, `deb`, or `rpm` according to the bundle type compiled
into the binary, so every Linux bundle type gets an explicit key. The bare
`linux-<arch>` keys stay pointed at the AppImage for clients that predate the
explicit keys.

Any missing asset, missing or empty signature, or signed desktop bundle left
out of the manifest exits non-zero with an `::error::` line, so a swallowed
failure can never ship a manifest with a blank signature or an unreachable
bundle.
"""
import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

# (manifest keys, regex that matches exactly one release asset)
LINUX_X64 = r'(amd64|x86_64)'
LINUX_ARM = r'(arm64|aarch64)'
TARGETS = [
    (['darwin-x86_64', 'darwin-aarch64'], r'\.app\.tar\.gz$'),
    (['windows-x86_64'], r'x64.*-setup\.exe$'),
    (['windows-aarch64'], r'arm64.*-setup\.exe$'),
    (['linux-x86_64', 'linux-x86_64-appimage'], LINUX_X64 + r'.*\.AppImage$'),
    (['linux-aarch64', 'linux-aarch64-appimage'], LINUX_ARM + r'.*\.AppImage$'),
    (['linux-x86_64-deb'], LINUX_X64 + r'.*\.deb$'),
    (['linux-aarch64-deb'], LINUX_ARM + r'.*\.deb$'),
    (['linux-x86_64-rpm'], LINUX_X64 + r'.*\.rpm$'),
    (['linux-aarch64-rpm'], LINUX_ARM + r'.*\.rpm$'),
]

# A signed asset with one of these endings is a desktop bundle and must be
# reachable through the manifest.
DESKTOP_BUNDLE = re.compile(r'(\.app\.tar\.gz|\.dmg|\.exe|\.msi|\.AppImage(\.tar\.gz)?|\.deb|\.rpm)$')


class ManifestError(Exception):
    pass


def read_assets(path):
    names = []
    for line in Path(path).read_text().splitlines():
        if line.strip():
            names.append(line.split('\t')[0])
    return names


def exactly_one(names, pattern):
    matches = [n for n in names if not n.endswith('.sig') and re.search(pattern, n)]
    if len(matches) != 1:
        raise ManifestError(f"Expected exactly one asset matching '{pattern}', got: {matches}")
    return matches[0]


def signature_for(asset, names, sig_dir):
    sig_name = asset + '.sig'
    if sig_name not in names:
        raise ManifestError(f'Missing {sig_name} on the release.')
    path = Path(sig_dir) / sig_name
    if not path.is_file():
        raise ManifestError(f'{sig_name} is on the release but was not downloaded to {sig_dir}.')
    signature = path.read_text().strip()
    if not signature:
        raise ManifestError(f'{sig_name} is empty.')
    return signature


def changelog_notes(changelog_path, version):
    """The version's section of CHANGELOG.md as Markdown, heading excluded.

    Empty when there is no such section (pre-release tags, for one); the
    caller substitutes the version name."""
    if not changelog_path or not Path(changelog_path).is_file():
        return ''
    heading = f'## [{version}]'
    lines = Path(changelog_path).read_text().splitlines()
    out, inside = [], False
    for line in lines:
        if line.startswith('## ['):
            if inside:
                break
            inside = line.startswith(heading)
            continue
        if inside:
            out.append(line)
    return '\n'.join(out).strip()


def build(repo, tag, names, sig_dir, changelog=None, pub_date=None):
    version = tag[1:] if tag.startswith('v') else tag
    notes = changelog_notes(changelog, version)
    if not notes:
        # Never ship blank notes, and never let it pass unnoticed: a release
        # tagged before its CHANGELOG heading exists lands here too.
        print(f'::warning::CHANGELOG.md has no section for {version}; '
              'latest.json notes fall back to the version name.', file=sys.stderr)
        notes = f'Hew {version}'

    def url(asset):
        return f'https://github.com/{repo}/releases/download/{tag}/{asset}'

    platforms = {}
    for keys, pattern in TARGETS:
        asset = exactly_one(names, pattern)
        entry = {'url': url(asset), 'signature': signature_for(asset, names, sig_dir)}
        for key in keys:
            platforms[key] = dict(entry)
    manifest = {
        'version': version,
        'notes': notes,
        'pub_date': pub_date or datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'platforms': platforms,
    }
    guard(manifest, repo, tag, names, sig_dir)
    return manifest


def guard(manifest, repo, tag, names, sig_dir):
    """Every signed desktop bundle on the release is in the manifest under
    exactly its own URL and signature. A bundle that was signed but has no
    key would leave some client with nothing to install (or, worse, with a
    different bundle type's file)."""
    listed = {(p['url'], p['signature']) for p in manifest['platforms'].values()}
    problems = []
    for name in names:
        if not name.endswith('.sig'):
            continue
        asset = name[: -len('.sig')]
        if not DESKTOP_BUNDLE.search(asset):
            continue
        want = (
            f'https://github.com/{repo}/releases/download/{tag}/{asset}',
            (Path(sig_dir) / name).read_text().strip() if (Path(sig_dir) / name).is_file() else None,
        )
        if want not in listed:
            problems.append(f'{asset} is signed but latest.json has no key pointing at it with its signature.')
    for key, entry in manifest['platforms'].items():
        if not entry['signature']:
            problems.append(f'{key} has a blank signature.')
    if problems:
        raise ManifestError(' '.join(problems))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--repo', required=True)
    ap.add_argument('--tag', required=True)
    ap.add_argument('--assets', required=True)
    ap.add_argument('--sigs', required=True)
    ap.add_argument('--changelog')
    ap.add_argument('--out', default='latest.json')
    ap.add_argument('--pub-date')
    args = ap.parse_args(argv)
    try:
        manifest = build(args.repo, args.tag, read_assets(args.assets), args.sigs,
                         args.changelog, args.pub_date)
    except ManifestError as err:
        print(f'::error::{err}', file=sys.stderr)
        return 1
    Path(args.out).write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())
