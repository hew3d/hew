# Distribution packaging

Everything here repackages the release artifacts GitHub Actions already
produces (`.github/workflows/release.yml`) — the Linux `.deb` for x86_64
and aarch64, and the notarized universal macOS `.dmg` — for platforms
whose users asked for a native package.
Nothing is built from source here: the desktop build needs pnpm, wasm-pack
and a pinned wasm-opt at build time, and none of these ecosystems can
express that offline dependency chain cleanly, so every package below is a
`-bin` package by design.

All of it is produced and published by `.github/workflows/packaging.yml`,
which runs whenever a GitHub release is published. Nothing here is ever
run or pushed by hand.

| Directory | What it is | Where it gets published |
|---|---|---|
| `pacman/hew-bin/` | a `PKGBUILD` build recipe, built by CI (never AUR — AUR has stopped accepting new maintainer registrations, and this route is a strictly better user experience anyway: a real signed binary repo, no local build) | a self-hosted, GPG-signed pacman repository under `hew3d/hew-packages` (GitHub Pages) |
| `gentoo/` | a Gentoo overlay: `media-gfx/hew-bin` ebuild, `metadata.xml`, `Manifest`, and the overlay's own `profiles/` and `metadata/` | the `hew3d/gentoo-overlay` repository, added with `eselect repository add hew3d git https://github.com/hew3d/gentoo-overlay.git` |
| `flatpak/` | a Flatpak manifest (`com.hew3d.Hew.yml`) and AppStream metainfo | a self-hosted, GPG-signed Flatpak repo under `hew3d/hew-packages`, plus a single-file `.flatpak` bundle attached to each GitHub release |
| `homebrew/` | a Homebrew cask (`Casks/hew.rb`) that installs the release `.dmg` and links `hew-cli` onto `PATH`, plus the tap's own `README.md` | the `hew3d/homebrew-tap` repository, installed with `brew install --cask hew3d/tap/hew` |
| `pages/index.html` | the landing page for `hew3d.github.io/hew-packages`, with install and update instructions for all three | published as-is alongside the pacman/Flatpak repos |
| `scripts/` | the render/build/validate scripts the workflow calls; see the header comment in each for what it does | n/a — CI-only |

## Publishing pipeline

`packaging.yml` triggers on `release: published` (a release stays a draft
until a human publishes it, so this never fires on its own) or manually via
`workflow_dispatch` against an already-published tag, for testing without
cutting a new release. Job graph:

1. **render** — renders the version and checksums into every template
   (`scripts/render-templates.sh`) and publishes the result as a build
   artifact for the other jobs.
2. **pacman** (x86_64 only — mainline Arch has no aarch64 port) — builds
   and GPG-signs the package, then signs the repo database, inside an
   `archlinux` container.
3. **flatpak** — builds the Flatpak, GPG-signs the ostree repo, and
   produces a single-file `.flatpak` bundle.
4. **gentoo-validate** — actually unpacks and installs the rendered
   ebuild inside a `gentoo/stage3` container, so a broken Manifest digest
   or `src_install` fails here instead of on a user's machine.
5. **publish-pages** — assembles the pacman + Flatpak output into
   `hew3d/hew-packages` (the GitHub Pages host) and republishes it
   wholesale, then attaches the `.flatpak` bundle to the GitHub release.
6. **gentoo-publish** — after validation passes, republishes
   `packaging/gentoo/` wholesale into `hew3d/gentoo-overlay`.
7. **homebrew-validate** — lints and audits the rendered cask, then
   installs it for real on a macOS runner (the `.dmg` download and
   checksum, the app, and the `hew-cli` link), then uninstalls it with
   `--zap` under a scratch `HOME` seeded with stand-ins for user work that
   must survive (`scripts/validate-homebrew-cask.sh`). If the release has
   no `.dmg`, render leaves the cask unrendered and only the two Homebrew
   jobs fail; the Linux targets still publish.
8. **homebrew-publish** — after validation passes, republishes
   `packaging/homebrew/` wholesale into `hew3d/homebrew-tap`.
9. **flatpak-update-smoke** — after the Pages publish, installs the previous
   release's Flatpak bundle, runs `flatpak update`, and checks the installed
   version becomes the new one (and that a fresh install gets it). Reporting
   only.

`hew3d/hew-packages`, `hew3d/gentoo-overlay`, and `hew3d/homebrew-tap` are
all rebuilt and
republished **wholesale** on every run rather than incrementally appended
to — there's no persistent ostree object store or pacman package history to
grow unbounded, at the cost of not supporting downgrades or Flatpak delta
updates. That's the right trade at Hew's current scale; revisit if either
ecosystem's users start asking for version history.

## One-time setup (not automatable — these are account- and repo-level
actions someone with the right access has to do once)

- Create `hew3d/hew-packages` (public), `hew3d/gentoo-overlay`, and
  `hew3d/homebrew-tap` (public, default branch `main`) if any of them
  doesn't already exist. Homebrew requires the `homebrew-` name prefix for
  `hew3d/tap` to resolve. Anything committed to these by hand is
  overwritten on the next run.
- Enable GitHub Pages on `hew3d/hew-packages`: Settings → Pages → Deploy
  from a branch → `main` / root.
- Generate a dedicated GPG signing keypair (no passphrase, since CI signs
  unattended) and add its exported private key as the
  `PACKAGING_GPG_PRIVATE_KEY` secret on `hew3d/hew` — and back the private
  key up somewhere durable and offline. This key is never regenerated by
  CI; losing it means re-keying and breaking trust for every existing
  pacman/Flatpak install until they re-add the remote.
- Create a fine-grained Personal Access Token scoped to exactly
  `hew3d/hew-packages`, `hew3d/gentoo-overlay`, and `hew3d/homebrew-tap`
  with Contents: Read and write, and add it as the `HEW_PACKAGES_TOKEN`
  secret on `hew3d/hew`.

## Flatpak runtime and install scope

Hew's remote contains the app; Flathub supplies its GNOME runtime. The
published install commands add both remotes with `--user` and install Hew
in that same scope, including on systems where Flathub is only configured
per user. Bundles name Flathub's `.flatpakrepo` as their runtime source and
Hew's raw repository URL as their update origin, with the public signing
key embedded so updates can be verified.

The command regression tests run without Flatpak or signing secrets:

```sh
python3 -B -m unittest discover -s packaging/tests -v
```

They execute the build script and published install commands against
controlled tool doubles; the real Linux build remains in the packaging
workflow.

## How the app knows which package manager updates it

Every Linux package here repackages the release `.deb`, so its binary carries
the Deb bundle marker that Tauri's updater uses to pick an installer. Left
alone, a Flatpak, pacman, or Gentoo copy would try to run `dpkg -i`. The app
(`shells/tauri/src-tauri/src/install_channel.rs`) decides instead, per copy,
first match wins:

1. Inside a Flatpak sandbox (`/.flatpak-info` or `FLATPAK_ID`): `flatpak`.
2. A marker file `share/hew/install-channel` next to the binary's `bin/`
   directory. Its first line names the channel. Each recipe writes it:

   | Recipe | Writes | Where |
   |---|---|---|
   | `flatpak/com.hew3d.Hew.yml` | `flatpak` | `/app/share/hew/install-channel` |
   | `pacman/hew-bin/PKGBUILD` | `pacman` | `/usr/share/hew/install-channel` |
   | `gentoo/media-gfx/hew-bin/*.ebuild` | `gentoo` | `/usr/share/hew/install-channel` |

   Any other non-empty value means "some other package manager": the app
   only announces updates. A third-party packager who repackages Hew should
   write its own name there.
3. The bundle marker in the binary. An AppImage updates itself. A Deb or Rpm
   marker updates itself, after a password prompt, only when `dpkg-query -S`
   or `rpm -qf` says the package manager owns the running binary; a
   repackager the app has never heard of is treated as "some other package
   manager" instead of being handed to `dpkg -i`.
4. Anything else (a dev build) only announces.

For the channels that only announce, the app shows the package manager's
command (`flatpak update com.hew3d.Hew`, `sudo pacman -Syu`, `sudo emaint
sync -r hew3d && sudo emerge --update media-gfx/hew-bin`) and a link to
`hew3d.com/download/update-help`, which `site/src/pages/download/update-help.astro`
serves. The Homebrew cask is untouched: it installs the notarized `.dmg`
and its macOS updater.

`tests/test_install_channel.py` runs each recipe's marker step into a
scratch prefix, and `validate-gentoo-ebuild.sh` checks the installed
ebuild image. After each release's packages publish, the `flatpak-update-smoke`
job in `packaging.yml` (`scripts/smoke-flatpak-update.sh`) installs the
previous release's bundle, updates it, and checks a fresh install reports the
new version. It only reports; it never gates or undoes a publish.

The release's `latest.json` is assembled by
`.github/scripts/build-updater-manifest.py` (tested by
`tests/test_updater_manifest.py`). It lists the `.deb` and `.rpm` for each
architecture under `linux-<arch>-deb` and `linux-<arch>-rpm`, the AppImage under
`linux-<arch>-appimage` and the bare `linux-<arch>`, and fails the release job if
any signed desktop bundle is left out.

## Why the cask keeps the in-app updater

A cask installs a prebuilt app by definition, so the cask uses the same
signed and notarized `.dmg` the download page serves, updater included,
rather than a second updater-free build that would need its own universal
build and notarization on every release. It declares `auto_updates true`,
which is how Homebrew treats self-updating apps: the app replaces itself
in place, and `brew upgrade` leaves it alone unless run with `--greedy`.
The tap still moves to each new release, so fresh installs always get the
current version.

## Why not a source package

Arch's `makepkg` allows network access during `prepare()`, so a source
`PKGBUILD` is possible there; Gentoo's `cargo.eclass` wants every crate
listed up front and has no story for a pnpm workspace or for wasm-pack
fetching `wasm-bindgen`/`wasm-opt`, and Gentoo's build sandbox blocks
network access entirely during compilation (unlike Arch's `prepare()`);
Flathub prefers source builds for open-source apps and would need
generated `cargo-sources.json` and node-sources manifests plus vendored
wasm tooling. The `-bin` route ships today, self-hosted rather than
through either ecosystem's official channel (Hew doesn't have the
traction yet for that to be worth pursuing) — a from-source route is a
separate effort if it's ever warranted.
