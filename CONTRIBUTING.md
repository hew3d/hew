# Contributing to Hew

Thanks for your interest in Hew. The project is young and moving quickly,
which makes focused, well-tested contributions especially valuable — and
makes coordination before large changes essential.

## Before you start

- Read [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md). It covers setup,
  repository layout, and the project's non-negotiable engineering rules.
  Code throughout the kernel cites those rules by number.
- Check [docs/ROADMAP.md](docs/ROADMAP.md) and the issue tracker. For
  anything larger than a small fix, open an issue first and outline your
  approach — architectural direction (kernel dependencies, the file
  format, the public WASM surface) is deliberately conservative, and
  agreeing on the shape of a change before you build it saves everyone
  time.
- Geometry kernel work is **spec-first**: operations land as typed
  signatures with executable specs and property tests before or alongside
  their implementation. If your change adds a kernel operation another
  piece of work will consume, land the agreed signature and spec early.

## Making a change

1. Fork and branch from `main`.
2. Keep the change focused. Unrelated refactors and reformatting belong in
   their own PRs — wholesale reformatting of shared files guarantees
   conflicts.
3. Add tests. New kernel mutations ship with topology-validator coverage
   and property tests in the same PR; UI changes ship with component tests.
   Never weaken or delete a failing test to get green.
4. If you touch serialization, update
   [docs/HEW_FILE_FORMAT.md](docs/HEW_FILE_FORMAT.md) in the same commit.
5. Run the verification gate and make sure it passes:

   ```sh
   scripts/verify.sh
   ```

   It runs the workspace build, all tests, `clippy -D warnings`, and
   formatting checks. CI runs the same gate; a PR that fails it will not
   be reviewed.

## Pull requests

- Use [Conventional Commits](https://www.conventionalcommits.org/) titles
  (`feat(kernel): …`, `docs: …`) and write the body for a human reader:
  what the change does and why, not a file-by-file narration.
- Small, reviewable PRs merge fast. A PR that mixes a kernel change with
  UI work will usually be asked to split.
- Expect review to focus on invariants: watertightness, validator
  coverage, tolerance discipline, and the purity boundaries between
  crates.

## The licensing wall

Hew imports SketchUp files via [OpenSKP](https://github.com/hew3d/openskp),
a clean-room implementation. To keep that position defensible, **nothing
derived from the SketchUp SDK — headers, constants, or format knowledge
obtained under Trimble's license — may enter this repository or its
dependency chain.** If you have ever accepted the SketchUp SDK license,
do not contribute to the `.skp` import path. This rule has no exceptions.

## Reporting bugs

File issues at [github.com/hew3d/hew/issues](https://github.com/hew3d/hew/issues).
The most useful reports include the `.hew` file, the debug log, and — for
anything interactive — a session recording; see
[docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) for where to find all three.
